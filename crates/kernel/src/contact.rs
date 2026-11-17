// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Public closed-mesh pair contact facts for the model checks (Q03 slice 2).
//!
//! Every fact here is decided from the exact orientation predicates of
//! [`crate::exact`], never from a tolerance: the relation of two closed,
//! consistently oriented shells is a property of their surfaces, and a
//! re-triangulation of either surface that keeps the vertices cannot change
//! it. What a caller does with the facts — which tolerance band a penetration
//! depth is compared against — is the caller's decision and stays outside
//! this module.
//!
//! Two ceilings are documented for the callers:
//!
//! - a point classification whose six axis rays all graze an edge or vertex is
//!   reported unresolved, never guessed;
//! - [`penetration_depth`] samples the inside shell's vertices, so it is a
//!   deterministic lower bound of the true penetration depth, invariant under
//!   re-triangulations that keep the vertex set.

use crate::bigint::Big;
use crate::exact::{PredError, Predicates, min_exponent, scaled};
use crate::intersect::{
    Side, Tri, classify_point, coplanar_area_overlap, segment_crosses_triangle, triangles_meet,
};
use crate::spatial::{Aabb, BoxIndex, IndexError};
use crate::topology::Limits;
use crate::trimesh::TriMesh;
use crate::verdict::MeshError;

/// Why a contact question was not decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactError {
    /// The exact-predicate budget of the run was used up.
    Budget,
    /// A value fell outside the fixed integer capacity of the predicates.
    Range,
    /// The spatial index refused the work (item, node, pair or result cap).
    Index(IndexError),
    /// A mesh coordinate problem (outside the coordinate caps, for example).
    Mesh(MeshError),
}

impl From<PredError> for ContactError {
    fn from(e: PredError) -> Self {
        match e {
            PredError::Budget => Self::Budget,
            PredError::Range => Self::Range,
        }
    }
}

impl From<MeshError> for ContactError {
    fn from(e: MeshError) -> Self {
        Self::Mesh(e)
    }
}

/// The relation of two closed, consistently oriented shells.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Relation {
    /// The closed surfaces are disjoint and neither interior meets the other.
    Separated,
    /// The closed surfaces meet, but no strict surface crossing and no
    /// interior overlap was proven: face contact, or an edge or vertex touch.
    Touching,
    /// The interiors overlap: a strict surface crossing, or full containment
    /// with disjoint surfaces.
    Overlapping {
        /// True when a strict surface crossing was proven; false for a
        /// containment without any surface meeting.
        surface_crossing: bool,
    },
}

/// The contact facts of one mesh pair.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PairReport {
    /// The relation of the two shells.
    pub relation: Relation,
    /// True when a coplanar face-to-face contact was seen: meeting triangles
    /// of one plane with opposite outward orientation. Face contact is a fact
    /// about the surfaces; how much area it covers is not decided here.
    pub face_contact: bool,
}

fn separated() -> PairReport {
    PairReport {
        relation: Relation::Separated,
        face_contact: false,
    }
}

/// The triangles of a mesh and their boxes, prepared once per shell.
struct Shell {
    tris: Vec<Tri>,
    boxes: Vec<Aabb>,
    bounds: Aabb,
}

impl Shell {
    fn of(mesh: &TriMesh) -> Result<Self, ContactError> {
        let vertices = mesh.vertices();
        let tri_at = |t: [u32; 3]| -> Result<Tri, ContactError> {
            let at = |i: u32| -> Result<[f64; 3], ContactError> {
                usize::try_from(i)
                    .ok()
                    .and_then(|i| vertices.get(i))
                    .copied()
                    .ok_or(ContactError::Range)
            };
            Ok([at(t[0])?, at(t[1])?, at(t[2])?])
        };
        let tris = mesh
            .triangles()
            .iter()
            .map(|&t| tri_at(t))
            .collect::<Result<Vec<_>, ContactError>>()?;
        let boxes = tris
            .iter()
            .map(|t| Aabb::from_points(t.iter().copied()))
            .collect::<Option<Vec<_>>>()
            .ok_or(ContactError::Range)?;
        let bounds =
            Aabb::from_points(mesh.vertices().iter().copied()).ok_or(ContactError::Range)?;
        Ok(Self {
            tris,
            boxes,
            bounds,
        })
    }
}

/// One strict point answer per shell, sampled over the other shell's
/// containment samples (vertices and face probes). Disjoint closed surfaces
/// are all-in or all-out, so mixed strict answers would mean a crossing this
/// scan missed. Corners and flush walls lie on the other shell's edge lines
/// more often than not, so the face probes are what decide the buried
/// cases; 4096 samples keep the work bounded.
fn sampled_contains(
    preds: &mut Predicates,
    shell: &Shell,
    other: &TriMesh,
) -> Result<Option<bool>, ContactError> {
    let mut seen: Option<bool> = None;
    for vertex in samples_of(other)?.into_iter().take(4096) {
        if let Some(inside) = shell_contains(preds, shell, vertex)? {
            if seen.is_some() && seen != Some(inside) {
                return Err(ContactError::Range);
            }
            seen = Some(inside);
        }
    }
    Ok(seen)
}

/// The contact facts of two closed, consistently oriented meshes in one frame.
///
/// Candidate triangle pairs are pruned with a box index over `a`'s triangles;
/// each candidate pair is decided with the exact predicates. Without any
/// meeting pair, one vertex of each shell decides between separation and full
/// containment, because disjoint closed surfaces are either fully apart or one
/// strictly inside the other.
pub fn contact(a: &TriMesh, b: &TriMesh, limits: Limits) -> Result<PairReport, ContactError> {
    let shell_a = Shell::of(a)?;
    let shell_b = Shell::of(b)?;
    if !shell_a.bounds.overlaps(&shell_b.bounds) {
        return Ok(separated());
    }
    let index = BoxIndex::build(&shell_a.boxes, limits.max_index_nodes, || false)
        .map_err(ContactError::Index)?;
    let mut preds = Predicates::new(limits.max_exact_predicates);
    let mut report = separated();
    let mut examined = 0u64;
    for (bi, query) in shell_b.boxes.iter().enumerate() {
        let Some(tb) = shell_b.tris.get(bi) else {
            return Err(ContactError::Range);
        };
        let candidates = index
            .overlapping(
                query,
                usize::try_from(limits.max_candidate_pairs).unwrap_or(0),
            )
            .map_err(ContactError::Index)?;
        for ai in candidates {
            let Some(ta) = shell_a.tris.get(usize::try_from(ai).unwrap_or(usize::MAX)) else {
                return Err(ContactError::Range);
            };
            examined += 1;
            if examined > limits.max_candidate_pairs {
                return Err(ContactError::Index(IndexError::PairCap));
            }
            if pair_facts(&mut preds, ta, tb, &mut report)? {
                // A strict crossing decided the pair relation; the remaining
                // candidates cannot change it. A face-contact flag recorded on
                // the way is kept in the report.
                return Ok(report);
            }
        }
    }
    if matches!(report.relation, Relation::Separated | Relation::Touching) {
        // A strict interior point decides what the surface meeting pattern
        // cannot: a flush-buried part meets the host only along coplanar
        // walls, its surface fact is Touching, and its interior still
        // overlaps. Exact face contact samples on-surface or outside and
        // keeps Touching; mixed strict answers under Separated stay the
        // defensive error they always were.
        let a_inside_b = sampled_contains(&mut preds, &shell_b, a)?;
        let b_inside_a = sampled_contains(&mut preds, &shell_a, b)?;
        if a_inside_b == Some(true) || b_inside_a == Some(true) {
            report.relation = Relation::Overlapping {
                surface_crossing: false,
            };
        } else if report.relation == Relation::Separated {
            match (a_inside_b, b_inside_a) {
                (Some(false), Some(false)) => {}
                _ => return Err(ContactError::Range),
            }
        }
    }
    Ok(report)
}

/// Decides one candidate pair; true when the pair strictly crosses.
fn pair_facts(
    preds: &mut Predicates,
    ta: &Tri,
    tb: &Tri,
    report: &mut PairReport,
) -> Result<bool, ContactError> {
    // Strict crossing: an edge of one triangle pierces the other's plane with
    // strictly opposite sides, and the closed sign pattern admits the point.
    for (s0, s1) in [(ta[0], ta[1]), (ta[1], ta[2]), (ta[2], ta[0])] {
        if segment_crosses_triangle(preds, s0, s1, tb)? {
            report.relation = Relation::Overlapping {
                surface_crossing: true,
            };
            return Ok(true);
        }
    }
    for (s0, s1) in [(tb[0], tb[1]), (tb[1], tb[2]), (tb[2], tb[0])] {
        if segment_crosses_triangle(preds, s0, s1, ta)? {
            report.relation = Relation::Overlapping {
                surface_crossing: true,
            };
            return Ok(true);
        }
    }
    if triangles_meet(preds, ta, tb)? {
        report.relation = Relation::Touching;
        if coplanar_opposite(preds, ta, tb)? {
            report.face_contact = true;
        }
    }
    Ok(false)
}

/// True when the two triangles lie in one exact plane, overlap in area, and
/// face opposite ways.
fn coplanar_opposite(preds: &mut Predicates, ta: &Tri, tb: &Tri) -> Result<bool, ContactError> {
    let [a0, a1, a2] = *ta;
    for point in tb {
        if preds.orient3d(a0, a1, a2, *point)? != 0 {
            return Ok(false);
        }
    }
    if !coplanar_area_overlap(preds, ta, tb)? {
        return Ok(false);
    }
    normals_opposite(ta, tb).map_err(ContactError::from)
}

/// The exact sign of `(b - a) x (c - a) . (e - d) x (f - d)`, the dot of the
/// two triangle normals, computed over the scaled binary64 inputs. Only the
/// sign is decided, so the common scale factor drops out.
fn normals_opposite(ta: &Tri, tb: &Tri) -> Result<bool, PredError> {
    let points = ta.iter().chain(tb.iter()).copied();
    let e = min_exponent(points.flatten());
    let big3 = |p: [f64; 3]| -> Result<[Big; 3], PredError> {
        let [x, y, z] = p;
        Ok([scaled(x, e)?, scaled(y, e)?, scaled(z, e)?])
    };
    let sub = |a: &[Big; 3], b: &[Big; 3]| -> Result<[Big; 3], PredError> {
        Ok([a[0].sub(&b[0])?, a[1].sub(&b[1])?, a[2].sub(&b[2])?])
    };
    let cross = |u: &[Big; 3], v: &[Big; 3]| -> Result<[Big; 3], PredError> {
        Ok([
            u[1].mul(&v[2])?.sub(&u[2].mul(&v[1])?)?,
            u[2].mul(&v[0])?.sub(&u[0].mul(&v[2])?)?,
            u[0].mul(&v[1])?.sub(&u[1].mul(&v[0])?)?,
        ])
    };
    let [a0, a1, a2] = [big3(ta[0])?, big3(ta[1])?, big3(ta[2])?];
    let [b0, b1, b2] = [big3(tb[0])?, big3(tb[1])?, big3(tb[2])?];
    let na = cross(&sub(&a1, &a0)?, &sub(&a2, &a0)?)?;
    let nb = cross(&sub(&b1, &b0)?, &sub(&b2, &b0)?)?;
    let dot = na[0]
        .mul(&nb[0])?
        .add(&na[1].mul(&nb[1])?)?
        .add(&na[2].mul(&nb[2])?)?;
    Ok(dot.sign() < 0)
}

/// Position of one point against one mesh: `Some(true)` strictly inside,
/// `Some(false)` strictly outside, `None` on the surface or unresolved.
pub fn contains(
    mesh: &TriMesh,
    point: [f64; 3],
    limits: Limits,
) -> Result<Option<bool>, ContactError> {
    let shell = Shell::of(mesh)?;
    let mut preds = Predicates::new(limits.max_exact_predicates);
    shell_contains(&mut preds, &shell, point)
}

fn shell_contains(
    preds: &mut Predicates,
    shell: &Shell,
    point: [f64; 3],
) -> Result<Option<bool>, ContactError> {
    match classify_point(preds, &shell.tris, point)? {
        Side::Inside => Ok(Some(true)),
        Side::Outside => Ok(Some(false)),
        Side::OnSurface | Side::Unresolved => Ok(None),
    }
}

/// The deepest strictly-contained sample of `inside`, measured as its distance
/// to `shell`'s surface, or `None` when no sample lies strictly inside.
///
/// The samples are the vertices of `inside` and, per triangle, the midpoint
/// between one corner and the midpoint of the opposite edge — a point strictly
/// inside the face by convexity with exactly representable coordinates, which
/// reaches the interior of aligned prisms whose vertices all lie on the
/// shell's walls. The value is a deterministic lower bound of the true
/// penetration depth, invariant under re-triangulations that keep the vertex
/// set. At most [`Limits::max_candidate_pairs`] contained samples are
/// measured, in mesh order, so a larger shell keeps the work bounded.
pub fn penetration_depth(
    inside: &TriMesh,
    shell: &TriMesh,
    limits: Limits,
) -> Result<Option<f64>, ContactError> {
    let shell = Shell::of(shell)?;
    let mut preds = Predicates::new(limits.max_exact_predicates);
    let mut deepest: Option<f64> = None;
    let mut measured = 0u64;
    for sample in &samples_of(inside)? {
        if !shell.bounds.contains_point(*sample) {
            continue;
        }
        if shell_contains(&mut preds, &shell, *sample)? != Some(true) {
            continue;
        }
        measured += 1;
        if measured > limits.max_candidate_pairs {
            break;
        }
        let depth = point_shell_closest(*sample, &shell).0;
        deepest = Some(match deepest {
            Some(best) if best >= depth => best,
            _ => depth,
        });
    }
    Ok(deepest)
}

/// The measured distance from a point to a mesh's triangles, in the meshes'
/// coordinate units (millimetres in the product). A measurement over the
/// given triangles, not a predicate: a faceted arc reads its chord deviation,
/// and the caller's tolerance band absorbs that bound.
pub fn point_gap(point: [f64; 3], mesh: &TriMesh) -> Result<f64, ContactError> {
    Ok(point_shell_closest(point, &Shell::of(mesh)?).0)
}

/// The measured distance from a point to a mesh's triangles and the closest
/// surface point, in the meshes' coordinate units (millimetres in the
/// product). Like [`point_gap`], a measurement over the given triangles, not
/// a predicate: the closest point is where that measurement is attained.
pub fn point_gap_closest(point: [f64; 3], mesh: &TriMesh) -> Result<(f64, [f64; 3]), ContactError> {
    Ok(point_shell_closest(point, &Shell::of(mesh)?))
}

/// The measured minimum distance between the two meshes' surfaces, in their
/// coordinate units (millimetres in the product). The scan walks both meshes'
/// containment samples (vertices and face probes) against the other shell,
/// so the value is deterministic under re-triangulations that keep the
/// vertex set; like the depth measurement it is sample-based — a surface
/// region no sample reaches can only make the true distance smaller, never
/// larger, and the caller's uncertainty band absorbs the difference.
pub fn surface_gap(a: &TriMesh, b: &TriMesh, limits: Limits) -> Result<f64, ContactError> {
    let shell_a = Shell::of(a)?;
    let shell_b = Shell::of(b)?;
    let mut best = f64::INFINITY;
    let mut measured = 0u64;
    for (samples, shell) in [(samples_of(a)?, &shell_b), (samples_of(b)?, &shell_a)] {
        for sample in &samples {
            if measured >= limits.max_candidate_pairs {
                return Ok(best);
            }
            measured += 1;
            let depth = point_shell_closest(*sample, shell).0;
            if depth < best {
                best = depth;
            }
        }
    }
    Ok(best)
}

/// The containment samples of a mesh: its vertices and, per triangle, the
/// midpoint between one corner and the midpoint of the opposite edge — a
/// point strictly inside the face by convexity with exactly representable
/// coordinates, which reaches the interior of aligned prisms whose vertices
/// all lie on the other shell's walls.
fn samples_of(mesh: &TriMesh) -> Result<Vec<[f64; 3]>, ContactError> {
    let mut samples: Vec<[f64; 3]> = Vec::with_capacity(mesh.vertices().len());
    samples.extend_from_slice(mesh.vertices());
    for t in mesh.triangles() {
        let at = |i: u32| -> Result<[f64; 3], ContactError> {
            usize::try_from(i)
                .ok()
                .and_then(|i| mesh.vertices().get(i))
                .copied()
                .ok_or(ContactError::Range)
        };
        let (v0, v1, v2) = (at(t[0])?, at(t[1])?, at(t[2])?);
        samples.extend(face_probes(v0, v1, v2));
    }
    Ok(samples)
}

/// The distance from a point to a shell's triangles in `f64` and the closest
/// surface point (a measurement, not a predicate; the caller's tolerance band
/// absorbs its rounding).
fn point_shell_closest(point: [f64; 3], shell: &Shell) -> (f64, [f64; 3]) {
    let mut best = f64::INFINITY;
    let mut closest = point;
    for tri in &shell.tris {
        let (d2, at) = point_triangle_closest2(point, tri);
        if d2 < best {
            best = d2;
            closest = at;
        }
    }
    (best.sqrt(), closest)
}

/// Squared distance from a point to a triangle and the closest point on it
/// (the standard region walk).
fn point_triangle_closest2(p: [f64; 3], t: &Tri) -> (f64, [f64; 3]) {
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let (a, b, c) = (t[0], t[1], t[2]);
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(p, a));
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return (dot(ap, ap), a);
    }
    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return (dot(bp, bp), b);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        let foot = [a[0] + v * ab[0], a[1] + v * ab[1], a[2] + v * ab[2]];
        return (dot(sub(p, foot), sub(p, foot)), foot);
    }
    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return (dot(cp, cp), c);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        let foot = [a[0] + w * ac[0], a[1] + w * ac[1], a[2] + w * ac[2]];
        return (dot(sub(p, foot), sub(p, foot)), foot);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        let cb = sub(c, b);
        let foot = [b[0] + w * cb[0], b[1] + w * cb[1], b[2] + w * cb[2]];
        return (dot(sub(p, foot), sub(p, foot)), foot);
    }
    // Inside the triangle region: the distance to its plane, attained at the
    // perpendicular projection of the point.
    let n = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ];
    let nn = dot(n, n);
    if nn <= 0.0 {
        return (0.0, a);
    }
    let along = dot(ap, n);
    let foot = [
        p[0] - n[0] * (along / nn),
        p[1] - n[1] * (along / nn),
        p[2] - n[2] * (along / nn),
    ];
    (along * along / nn, foot)
}

/// The face probes of one triangle: for each corner, the midpoint between it
/// and the midpoint of the opposite edge. Every probe lies strictly inside the
/// triangle by convexity, and its coordinates are exact (only divisions by
/// two).
fn face_probes(v0: [f64; 3], v1: [f64; 3], v2: [f64; 3]) -> Vec<[f64; 3]> {
    let half = |u: [f64; 3], w: [f64; 3]| {
        [
            (u[0] + w[0]) / 2.0,
            (u[1] + w[1]) / 2.0,
            (u[2] + w[2]) / 2.0,
        ]
    };
    let probe = |corner: [f64; 3], edge: [f64; 3]| half(corner, edge);
    vec![
        probe(v0, half(v1, v2)),
        probe(v1, half(v2, v0)),
        probe(v2, half(v0, v1)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> Limits {
        Limits::DEFAULT
    }

    /// A cube as 12 consistently oriented triangles, the shared test vocabulary.
    /// A box shell over the two corners `lo` and `hi`, 12 consistently
    /// oriented triangles; the shared test vocabulary.
    fn shell_box(lo: [f64; 3], hi: [f64; 3]) -> TriMesh {
        let v = |i: usize| {
            [
                if i & 1 == 1 { hi[0] } else { lo[0] },
                if i & 2 == 2 { hi[1] } else { lo[1] },
                if i & 4 == 4 { hi[2] } else { lo[2] },
            ]
        };
        let tris: Vec<[u32; 3]> = [
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
        .into_iter()
        .collect();
        TriMesh::from_buffers(&(0..8).map(v).collect::<Vec<_>>(), &tris).unwrap()
    }

    fn cube(origin: [f64; 3], size: f64) -> TriMesh {
        shell_box(
            origin,
            [origin[0] + size, origin[1] + size, origin[2] + size],
        )
    }

    /// A square plate 200x200 with a square 60x60 through-hole in its
    /// centre (x, y in 70..130), 20 mm thick along z: an extrusion with one
    /// cavity contour, the same operation a holed model element evaluates
    /// through.
    fn holed_plate() -> TriMesh {
        let outer = [
            [-100.0, -100.0],
            [100.0, -100.0],
            [100.0, 100.0],
            [-100.0, 100.0],
        ];
        let hole = [[-30.0, -30.0], [-30.0, 30.0], [30.0, 30.0], [30.0, -30.0]];
        let square = |points: &[[f64; 2]]| {
            points
                .iter()
                .map(|[x, y]| crate::profile::Point2::new(*x, *y))
                .collect::<Vec<_>>()
        };
        let extrusion = crate::profile::extrude(
            &square(&outer),
            &[crate::profile::Cavity::polygon(square(&hole))],
            20.0,
        )
        .unwrap();
        let mesh = extrusion.mesh();
        TriMesh::from_buffers(
            &mesh
                .vertices()
                .iter()
                .map(|v| [v.x, v.y, v.z])
                .collect::<Vec<_>>(),
            mesh.triangles(),
        )
        .unwrap()
    }

    /// A square hollow box 200x200 outer, 100x100 inner cavity, 200 mm long
    /// along z (open at both ends): the extrusion of one cavity contour.
    fn cavity_box() -> TriMesh {
        let outer = [
            [-100.0, -100.0],
            [100.0, -100.0],
            [100.0, 100.0],
            [-100.0, 100.0],
        ];
        let hole = [[-50.0, -50.0], [-50.0, 50.0], [50.0, 50.0], [50.0, -50.0]];
        let square = |points: &[[f64; 2]]| {
            points
                .iter()
                .map(|[x, y]| crate::profile::Point2::new(*x, *y))
                .collect::<Vec<_>>()
        };
        let extrusion = crate::profile::extrude(
            &square(&outer),
            &[crate::profile::Cavity::polygon(square(&hole))],
            200.0,
        )
        .unwrap();
        let mesh = extrusion.mesh();
        TriMesh::from_buffers(
            &mesh
                .vertices()
                .iter()
                .map(|v| [v.x, v.y, v.z])
                .collect::<Vec<_>>(),
            mesh.triangles(),
        )
        .unwrap()
    }

    #[test]
    fn a_shaft_through_the_hole_is_separated_and_through_the_rim_overlaps() {
        // The hole is real missing material: a 50x50 shaft down the 60x60
        // hole with 5 mm of clearance on every side stays separated even
        // though it spans the plate's whole thickness and more.
        let plate = holed_plate();
        let shaft = cube([-25.0, -25.0, -40.0], 50.0);
        let report = contact(&plate, &shaft, limits()).unwrap();
        assert_eq!(report.relation, Relation::Separated);
        // The same shaft grown to 70x70 crosses the hole walls by 5 mm on
        // every side: a strict crossing of the surfaces.
        let fat = cube([-35.0, -35.0, -40.0], 70.0);
        let report = contact(&plate, &fat, limits()).unwrap();
        assert_eq!(
            report.relation,
            Relation::Overlapping {
                surface_crossing: true
            }
        );
    }

    #[test]
    fn an_insert_in_the_cavity_is_separated_and_through_the_wall_overlaps() {
        // The cavity walls are real material: an 80x80 insert floating
        // 10 mm from the cavity walls stays separated; a 110x110 insert
        // crosses the walls by 5 mm on every side.
        let box_shell = cavity_box();
        let floating = cube([-40.0, -40.0, 70.0], 80.0);
        let report = contact(&box_shell, &floating, limits()).unwrap();
        assert_eq!(report.relation, Relation::Separated);
        let fat = cube([-55.0, -55.0, 70.0], 110.0);
        let report = contact(&box_shell, &fat, limits()).unwrap();
        assert_eq!(
            report.relation,
            Relation::Overlapping {
                surface_crossing: true
            }
        );
    }

    #[test]
    fn separate_touching_and_overlapping_cubes_are_decided() {
        let a = cube([0.0; 3], 10.0);
        let far = cube([20.0; 3], 5.0);
        assert_eq!(contact(&a, &far, limits()).unwrap(), separated());
        // Exact face contact: stacked, shared plane, opposite orientation.
        let stacked = cube([0.0, 0.0, 10.0], 10.0);
        let report = contact(&a, &stacked, limits()).unwrap();
        assert_eq!(report.relation, Relation::Touching);
        assert!(report.face_contact);
        // Deep penetration: half into each other.
        let sunk = cube([5.0; 3], 10.0);
        let report = contact(&a, &sunk, limits()).unwrap();
        assert_eq!(
            report.relation,
            Relation::Overlapping {
                surface_crossing: true
            }
        );
        assert!(!report.face_contact);
    }

    #[test]
    fn containment_without_surface_meeting_is_overlapping() {
        let a = cube([0.0; 3], 10.0);
        let inner = cube([4.0; 3], 2.0);
        let report = contact(&a, &inner, limits()).unwrap();
        assert_eq!(
            report.relation,
            Relation::Overlapping {
                surface_crossing: false
            }
        );
        assert!(!report.face_contact);
    }

    #[test]
    fn edge_touch_is_touching_without_face_contact() {
        // Two cubes sharing exactly one vertical edge: touching, but the
        // shared feature is an edge, not a face.
        let a = cube([0.0; 3], 10.0);
        let b = cube([10.0, 10.0, 0.0], 10.0);
        let report = contact(&a, &b, limits()).unwrap();
        assert_eq!(report.relation, Relation::Touching);
        assert!(!report.face_contact);
        // Side-by-side cubes press identical wall squares together: coplanar,
        // overlapping in area, opposite orientations — face contact.
        let b = cube([10.0, 0.0, 0.0], 10.0);
        let report = contact(&a, &b, limits()).unwrap();
        assert_eq!(report.relation, Relation::Touching);
        assert!(report.face_contact);
    }

    #[test]
    fn depth_measures_the_deepest_contained_vertex() {
        let shell = cube([0.0; 3], 10.0);
        let inner = cube([4.0; 3], 2.0);
        // The inner corners sit 4 mm from the nearest wall set.
        assert_eq!(
            penetration_depth(&inner, &shell, limits()).unwrap(),
            Some(4.0)
        );
        let outside = cube([50.0; 3], 2.0);
        assert_eq!(penetration_depth(&outside, &shell, limits()).unwrap(), None);
    }

    #[test]
    fn an_aligned_prism_is_measured_by_its_face_probes() {
        // A plate whose every vertex lies on the shell's walls: the probes of
        // the face interiors still reach strictly inside.
        let wall = shell_box([0.0, 0.0, 0.0], [20.0, 20.0, 10.0]);
        let sunk = shell_box([2.0, 0.0, 5.0], [18.0, 20.0, 15.0]);
        assert_eq!(
            penetration_depth(&sunk, &wall, limits()).unwrap(),
            Some(5.0)
        );
        // The flush-buried case whose side walls poke through the shell's
        // top face: the surfaces genuinely cross along the meeting lines,
        // and the crossing is the decided fact.
        let report = contact(&wall, &sunk, limits()).unwrap();
        assert!(matches!(
            report.relation,
            Relation::Overlapping {
                surface_crossing: true
            }
        ));
        // Buried flush with one wall, fully inside the shell and placed so
        // that no edge of either shell strictly pierces the other (the
        // insert's flush-face region dodges both diagonals of the flush
        // wall): the surface facts alone say Touching, and the strict
        // interior point promotes the relation in the same call.
        let buried = shell_box([0.0, 12.0, 4.75], [8.0, 14.0, 5.25]);
        let report = contact(&wall, &buried, limits()).unwrap();
        assert_eq!(
            report.relation,
            Relation::Overlapping {
                surface_crossing: false
            }
        );
    }

    #[test]
    fn a_corner_overlap_stays_unresolved_or_shallow() {
        // The corner overlap's deepest face probes sit near the corner, so
        // this engineered case measures shallow or finds no sample at all;
        // the caller reports it indeterminate instead of guessing. The
        // relation itself is still a proven crossing.
        let a = cube([0.0; 3], 2.0);
        let b = cube([1.0; 3], 2.0);
        let report = contact(&a, &b, limits()).unwrap();
        assert!(matches!(
            report.relation,
            Relation::Overlapping {
                surface_crossing: true
            }
        ));
    }

    #[test]
    fn diagonal_flip_keeps_the_relation_and_the_depth() {
        let a = cube([0.0; 3], 10.0);
        // Off the symmetric diagonals: a corner exactly on the flipped face
        // diagonals leaves every axis ray grazing, which is the documented
        // unresolved ceiling of the axis-parity classification.
        let sunk = cube([8.0, 8.0, 8.5], 10.0);
        let plain = contact(&a, &sunk, limits()).unwrap();
        let depth = penetration_depth(&sunk, &a, limits()).unwrap();
        // The same vertices with every quad's diagonal flipped the other way:
        // a different, equally valid triangulation of the same surface.
        let flipped = {
            let v = a.vertices().to_vec();
            let tris: Vec<[u32; 3]> = [
                [1, 5, 4],
                [1, 4, 0], // face y=0
                [6, 7, 3],
                [6, 3, 2], // face y=1
                [2, 3, 1],
                [2, 1, 0], // face z=0
                [5, 7, 6],
                [5, 6, 4], // face z=1
                [4, 6, 2],
                [4, 2, 0], // face x=0
                [3, 7, 5],
                [3, 5, 1], // face x=1
            ]
            .into_iter()
            .collect();
            TriMesh::from_buffers(&v, &tris).unwrap()
        };
        assert_eq!(contact(&flipped, &sunk, limits()).unwrap(), plain);
        assert_eq!(penetration_depth(&sunk, &flipped, limits()).unwrap(), depth);
        assert!(matches!(depth, Some(d) if d > 0.0));
    }

    #[test]
    fn a_point_is_classified_or_left_unresolved() {
        let shell = cube([0.0; 3], 10.0);
        assert_eq!(
            contains(&shell, [3.7, 5.2, 6.1], limits()).unwrap(),
            Some(true)
        );
        assert_eq!(
            contains(&shell, [15.0, 5.0, 5.0], limits()).unwrap(),
            Some(false)
        );
        // On the surface is not strictly inside.
        assert_eq!(contains(&shell, [0.0, 5.0, 5.0], limits()).unwrap(), None);
    }

    #[test]
    fn point_gap_measures_face_edge_and_corner_distances() {
        let shell = cube([0.0; 3], 10.0);
        // Above the centre of the top face.
        let d = point_gap([5.0, 5.0, 12.0], &shell).unwrap();
        assert!((d - 2.0).abs() <= 1e-9, "{d}");
        // Off an edge: the nearest point is on the edge line.
        let d = point_gap([-3.0, 5.0, 10.0], &shell).unwrap();
        assert!((d - 3.0).abs() <= 1e-9, "{d}");
        // Off the corner: the nearest point is the corner itself.
        let d = point_gap([-3.0, -4.0, 10.0], &shell).unwrap();
        assert!((d - 5.0).abs() <= 1e-9, "{d}");
        // A point on the surface measures zero.
        assert_eq!(point_gap([5.0, 5.0, 10.0], &shell).unwrap(), 0.0);
    }

    #[test]
    fn surface_gap_measures_stacked_separated_and_corner_distances() {
        let a = cube([0.0; 3], 10.0);
        // Stacked with shared plane: the surfaces meet, gap zero.
        let stacked = cube([0.0, 0.0, 10.0], 10.0);
        assert_eq!(surface_gap(&a, &stacked, limits()).unwrap(), 0.0);
        // A uniform 5 mm air slab: every sampled distance is exact.
        let lifted = cube([0.0, 0.0, 15.0], 10.0);
        let d = surface_gap(&a, &lifted, limits()).unwrap();
        assert!((d - 5.0).abs() <= 1e-9, "{d}");
        // The nearest corner pair decides a diagonal offset.
        let shifted = cube([13.0, 0.0, 10.0], 10.0);
        let d = surface_gap(&a, &shifted, limits()).unwrap();
        assert!((d - 3.0).abs() <= 1e-9, "{d}");
    }

    #[test]
    fn point_gap_closest_names_the_surface_point_where_the_distance_is_attained() {
        let shell = cube([0.0; 3], 10.0);
        // Above the top face: the projection onto the face region.
        let (d, at) = point_gap_closest([5.0, 5.0, 12.0], &shell).unwrap();
        assert!((d - 2.0).abs() <= 1e-9, "{d}");
        assert!(
            (at[0] - 5.0).abs() <= 1e-9 && (at[1] - 5.0).abs() <= 1e-9,
            "{at:?}"
        );
        assert!((at[2] - 10.0).abs() <= 1e-9, "{at:?}");
        // Off an edge: the nearest point sits on the edge line.
        let (d, at) = point_gap_closest([-3.0, 5.0, 10.0], &shell).unwrap();
        assert!((d - 3.0).abs() <= 1e-9, "{d}");
        assert!(
            at[0].abs() <= 1e-9 && (at[1] - 5.0).abs() <= 1e-9 && (at[2] - 10.0).abs() <= 1e-9,
            "{at:?}"
        );
        // Off the corner: the corner itself.
        let (d, at) = point_gap_closest([-3.0, -4.0, 10.0], &shell).unwrap();
        assert!((d - 5.0).abs() <= 1e-9, "{d}");
        assert!(
            at[0].abs() <= 1e-9 && at[1].abs() <= 1e-9 && (at[2] - 10.0).abs() <= 1e-9,
            "{at:?}"
        );
        // A point on the surface is its own closest point at zero distance.
        let (d, at) = point_gap_closest([5.0, 5.0, 10.0], &shell).unwrap();
        assert_eq!(d, 0.0);
        assert!(
            (at[0] - 5.0).abs() <= 1e-9 && (at[1] - 5.0).abs() <= 1e-9,
            "{at:?}"
        );
    }
}
