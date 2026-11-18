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
//!   re-triangulations that keep the vertex set;
//! - [`crossing_depth`] reads the intersection itself for the case no sample
//!   reaches: the largest inside run of a strictly crossing surface edge,
//!   decided exactly and invariant under a different valid triangulation;
//! - [`coincident_depth`] reads a coincident face (#741) — a duplicate part,
//!   whose every sample lies on the other surface — from points proven
//!   strictly inside both shells next to the shared face.

use std::collections::BTreeMap;

use crate::bigint::Big;
use crate::exact::{PredError, Predicates, min_exponent, project, scaled};
use crate::intersect::{
    Pierce, Side, Tri, classify_point, coplanar_area_overlap, segment_pierces_triangle,
    triangles_meet,
};
use crate::spatial::{Aabb, BoxIndex, IndexError};
use crate::topology::Limits;
use crate::trimesh::TriMesh;
use crate::verdict::MeshError;
use std::collections::HashMap;

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
    /// The interiors overlap: a strict surface crossing, a full containment
    /// with disjoint surfaces, a flush burial, or a coincident face.
    Overlapping {
        /// True when a strict surface crossing was proven; false for an
        /// overlap proven without one (containment, flush burial, or a
        /// coincident face, see [`PairReport::coincident`]).
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
    /// True when a coincident face was seen: meeting triangles of one plane
    /// that overlap in area with the same outward orientation. Both closed
    /// shells keep their material on the inward side of that shared area, so
    /// the interiors overlap there — a duplicate or a flush part placed on
    /// top of another (#741). [`coincident_depth`] measures it.
    pub coincident: bool,
}

fn separated() -> PairReport {
    PairReport {
        relation: Relation::Separated,
        face_contact: false,
        coincident: false,
    }
}

/// The triangles of a mesh and their boxes, prepared once per shell.
///
/// Alongside the coordinates the shell keeps its vertex-index triples and
/// the edge adjacency: which triangles share each edge by index identity.
/// A boundary pierce counts as a crossing only through this continuation —
/// the shell's own surface continuing through the shared edge — so a touch
/// or a graze along a face plane stays a touch (#721).
struct Shell {
    tris: Vec<Tri>,
    boxes: Vec<Aabb>,
    bounds: Aabb,
    /// The mesh's triangle vertex indices, in mesh order: the identity the
    /// adjacency is keyed on.
    verts: Vec<[u32; 3]>,
    /// Every mesh vertex mapped to the triangles that contain it: the star
    /// of triangles around the vertex, the local surface a boundary pierce
    /// at that vertex has to pass through.
    vstar: HashMap<u32, Vec<usize>>,
    /// Every edge (sorted vertex-index pair) mapped to the triangles that
    /// contain it: the continuation of the surface across the edge.
    adj: HashMap<(u32, u32), Vec<usize>>,
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
        let mut adj: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
        let mut vstar: HashMap<u32, Vec<usize>> = HashMap::new();
        for (ti, t) in mesh.triangles().iter().enumerate() {
            for (u, v) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let key = (u.min(v), u.max(v));
                match adj.get_mut(&key) {
                    Some(faces) => faces.push(ti),
                    None => {
                        adj.insert(key, vec![ti]);
                    }
                }
                for vertex in [u, v] {
                    match vstar.get_mut(&vertex) {
                        Some(faces) => faces.push(ti),
                        None => {
                            vstar.insert(vertex, vec![ti]);
                        }
                    }
                }
            }
        }
        Ok(Self {
            tris,
            boxes,
            bounds,
            verts: mesh.triangles().to_vec(),
            vstar,
            adj,
        })
    }
}

/// What the containment samples read for one shell against the other.
#[derive(Debug, Clone, Copy, PartialEq)]
enum SampleScan {
    /// Every strict answer agreed: all outside, all inside, or none decided.
    Consistent(Option<bool>),
    /// Some samples read strictly inside and others strictly outside. For
    /// surfaces proven to meet, that is the flush-buried interpenetration:
    /// part of the body in, part out (#721). For surfaces proven disjoint it
    /// stays the defensive error it always was — a crossing the scan missed.
    Mixed,
}

/// One strict point answer per shell, sampled over the other shell's
/// containment samples (vertices and face probes). Corners and flush walls
/// lie on the other shell's edge lines more often than not, so the face
/// probes are what decide the buried cases; 4096 samples keep the work
/// bounded.
fn sampled_contains(
    preds: &mut Predicates,
    shell: &Shell,
    other: &TriMesh,
) -> Result<SampleScan, ContactError> {
    let mut seen: Option<bool> = None;
    for vertex in samples_of(other)?.into_iter().take(4096) {
        if let Some(inside) = shell_contains(preds, shell, vertex)? {
            if seen.is_some() && seen != Some(inside) {
                return Ok(SampleScan::Mixed);
            }
            seen = Some(inside);
        }
    }
    Ok(SampleScan::Consistent(seen))
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
        if shell_b.tris.get(bi).is_none() {
            return Err(ContactError::Range);
        }
        let candidates = index
            .overlapping(
                query,
                usize::try_from(limits.max_candidate_pairs).unwrap_or(0),
            )
            .map_err(ContactError::Index)?;
        for ai in candidates {
            let ta_i = usize::try_from(ai).unwrap_or(usize::MAX);
            if shell_a.tris.get(ta_i).is_none() {
                return Err(ContactError::Range);
            }
            examined += 1;
            if examined > limits.max_candidate_pairs {
                return Err(ContactError::Index(IndexError::PairCap));
            }
            if pair_facts(&mut preds, &shell_a, ta_i, &shell_b, bi, &mut report)? {
                // A strict crossing decided the pair relation; the remaining
                // candidates cannot change it. A face-contact flag recorded on
                // the way is kept in the report.
                return Ok(report);
            }
        }
    }
    if report.coincident && report.relation == Relation::Touching {
        // A coincident face keeps both bodies' material on the same side of
        // the shared area: the interiors overlap without a strict surface
        // crossing (#741). No sample scan is needed to prove it.
        report.relation = Relation::Overlapping {
            surface_crossing: false,
        };
        return Ok(report);
    }
    if matches!(report.relation, Relation::Separated | Relation::Touching) {
        // A strict interior point decides what the surface meeting pattern
        // cannot: a flush-buried part meets the host only along coplanar
        // walls and rim lines, its surface fact is Touching, and its
        // interior still overlaps — the samples then read mixed, some in
        // and some out, and that mixed scan is the proof (#721). Exact face
        // contact samples on-surface or outside and keeps Touching; mixed
        // strict answers under Separated stay the defensive error they
        // always were, because disjoint closed surfaces are all-in or
        // all-out.
        let a_inside_b = sampled_contains(&mut preds, &shell_b, a)?;
        let b_inside_a = sampled_contains(&mut preds, &shell_a, b)?;
        let inside = matches!(a_inside_b, SampleScan::Mixed)
            || a_inside_b == SampleScan::Consistent(Some(true))
            || matches!(b_inside_a, SampleScan::Mixed)
            || b_inside_a == SampleScan::Consistent(Some(true));
        if inside {
            report.relation = Relation::Overlapping {
                surface_crossing: false,
            };
        } else if report.relation == Relation::Separated {
            match (a_inside_b, b_inside_a) {
                (SampleScan::Consistent(Some(false)), SampleScan::Consistent(Some(false))) => {}
                _ => return Err(ContactError::Range),
            }
        }
    }
    Ok(report)
}

/// Decides one candidate pair; true when the pair strictly crosses.
fn pair_facts(
    preds: &mut Predicates,
    a: &Shell,
    ai: usize,
    b: &Shell,
    bi: usize,
    report: &mut PairReport,
) -> Result<bool, ContactError> {
    let ta = a.tris.get(ai).ok_or(ContactError::Range)?;
    let tb = b.tris.get(bi).ok_or(ContactError::Range)?;
    // Strict crossing: an edge of one triangle pierces the other's plane with
    // strictly opposite sides, and the exact sign pattern admits the point.
    // An interior pierce decides; a boundary pierce counts only when the
    // pierced shell's own surface continues through the shared edge and the
    // segment passes through that continuation transversally (#721): a
    // segment grazing along a face plane, or stopping at a perimeter edge,
    // touches the shell — it does not cross it.
    for (s0, s1) in [(ta[0], ta[1]), (ta[1], ta[2]), (ta[2], ta[0])] {
        if pierce_decides_crossing(preds, b, bi, s0, s1)? {
            report.relation = Relation::Overlapping {
                surface_crossing: true,
            };
            return Ok(true);
        }
    }
    for (s0, s1) in [(tb[0], tb[1]), (tb[1], tb[2]), (tb[2], tb[0])] {
        if pierce_decides_crossing(preds, a, ai, s0, s1)? {
            report.relation = Relation::Overlapping {
                surface_crossing: true,
            };
            return Ok(true);
        }
    }
    if triangles_meet(preds, ta, tb)? {
        report.relation = Relation::Touching;
        match coplanar_orientation(preds, ta, tb)? {
            Coplanar::Opposite => report.face_contact = true,
            // A coincident face proves an interior overlap, but the scan
            // goes on: a strict crossing found by a later pair still decides
            // the relation as a surface crossing (#741).
            Coplanar::Same => report.coincident = true,
            Coplanar::No => {}
        }
    }
    Ok(false)
}

/// One edge `s0 s1` of a foreign shell against the triangle `ti` of `shell`:
/// true when the pierce proves a strict crossing of `shell`'s surface. An
/// interior pierce proves it on its own. A boundary pierce — the pierce point
/// exactly on the triangle's edge or vertex — proves it only when the local
/// surface around the point confirms it: the star of triangles sharing the
/// edge (or, at a vertex, the triangles around that vertex), each plane of
/// which the segment must pass through transversally at the point (#721). A
/// segment running along one of those face planes — the diagonal of a
/// contact face grazing a wall at its bottom edge — or leaving the surface at
/// a rim, where the star has no continuation, touches the shell; it does not
/// cross it.
fn pierce_decides_crossing(
    preds: &mut Predicates,
    shell: &Shell,
    ti: usize,
    s0: [f64; 3],
    s1: [f64; 3],
) -> Result<bool, ContactError> {
    let tri = shell.tris.get(ti).ok_or(ContactError::Range)?;
    let verts = shell.verts.get(ti).ok_or(ContactError::Range)?;
    match segment_pierces_triangle(preds, s0, s1, tri).map_err(ContactError::from)? {
        Pierce::None => Ok(false),
        Pierce::Interior => Ok(true),
        Pierce::Boundary(mask) => {
            // Locate the pierce point by index identity: on exactly one edge
            // it is strictly inside that edge, and the local surface is the
            // edge's faces; on two edges it is their shared vertex, and the
            // local surface is the whole vertex star. One-bit and two-bit
            // masks are the only shapes a point on a non-degenerate triangle
            // takes; anything else would mean corrupt adjacency.
            let edges: Vec<usize> = (0..3).filter(|j| mask & (1 << j) != 0).collect();
            let vert = |i: usize| -> Result<u32, ContactError> {
                verts.get(i).copied().ok_or(ContactError::Range)
            };
            let vertex = match edges.as_slice() {
                [only] => vert(*only)?,
                [first, second, ..] => {
                    let a = [vert(*first)?, vert((first + 1) % 3)?];
                    let b = [vert(*second)?, vert((second + 1) % 3)?];
                    if a[0] == b[0] || a[0] == b[1] {
                        a[0]
                    } else {
                        a[1]
                    }
                }
                _ => return Err(ContactError::Range),
            };
            let star = if edges.len() == 1 {
                let edge = edges.first().copied().ok_or(ContactError::Range)?;
                let key = {
                    let (u, v) = (vert(edge)?, vert((edge + 1) % 3)?);
                    (u.min(v), u.max(v))
                };
                shell.adj.get(&key).ok_or(ContactError::Range)?
            } else {
                shell.vstar.get(&vertex).ok_or(ContactError::Range)?
            };
            // The segment must pass through every face of the local surface
            // transversally at the point. Its own triangle's plane it crosses
            // by the pierce precondition; a face whose plane carries the
            // segment, or whose plane the segment fails to leave on the far
            // side, vetoes the crossing.
            let mut continued = false;
            for &nf in star.iter() {
                if nf == ti {
                    continue;
                }
                let face = shell.tris.get(nf).ok_or(ContactError::Range)?;
                if !crosses_transversally(preds, face, s0, s1).map_err(ContactError::from)? {
                    return Ok(false);
                }
                continued = true;
            }
            Ok(continued)
        }
    }
}

/// True when the segment's endpoints lie strictly on opposite sides of the
/// triangle's plane.
fn crosses_transversally(
    preds: &mut Predicates,
    tri: &Tri,
    s0: [f64; 3],
    s1: [f64; 3],
) -> Result<bool, PredError> {
    let [a, b, c] = *tri;
    let d0 = preds.orient3d(a, b, c, s0)?;
    let d1 = preds.orient3d(a, b, c, s1)?;
    Ok(d0 != 0 && d1 != 0 && (d0 > 0) != (d1 > 0))
}

/// The plain `f64` orientation determinant of a point against a triangle's
/// plane — a measurement, not a predicate: the caller reads only a sample
/// coordinate from it, and the exact containment classification decides that
/// sample's fate.
fn orient3d_value(a: [f64; 3], b: [f64; 3], c: [f64; 3], p: [f64; 3]) -> f64 {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let ap = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
    let n = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ];
    n[0] * ap[0] + n[1] * ap[1] + n[2] * ap[2]
}

/// How two meeting triangles share a plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Coplanar {
    /// Not one exact plane, or no overlap in area.
    No,
    /// One plane, overlapping in area, facing opposite ways: face contact.
    Opposite,
    /// One plane, overlapping in area, facing the same way: a coincident face.
    Same,
}

/// Whether the two triangles lie in one exact plane and overlap in area, and
/// if so whether their outward normals face opposite ways (face contact) or
/// the same way (a coincident face).
fn coplanar_orientation(
    preds: &mut Predicates,
    ta: &Tri,
    tb: &Tri,
) -> Result<Coplanar, ContactError> {
    let [a0, a1, a2] = *ta;
    for point in tb {
        if preds.orient3d(a0, a1, a2, *point)? != 0 {
            return Ok(Coplanar::No);
        }
    }
    if !coplanar_area_overlap(preds, ta, tb)? {
        return Ok(Coplanar::No);
    }
    Ok(match normals_dot_sign(ta, tb)? {
        s if s < 0 => Coplanar::Opposite,
        s if s > 0 => Coplanar::Same,
        // Both triangles are non-degenerate and coplanar, so their normals
        // are parallel and the dot is never zero; stay conservative anyway.
        _ => Coplanar::No,
    })
}

/// The exact sign of `(b - a) x (c - a) . (e - d) x (f - d)`, the dot of the
/// two triangle normals, computed over the scaled binary64 inputs. Only the
/// sign is decided, so the common scale factor drops out.
fn normals_dot_sign(ta: &Tri, tb: &Tri) -> Result<i8, PredError> {
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
    Ok(dot.sign())
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

/// Position of one point against one mesh, closed: `Some(true)` strictly
/// inside or exactly on the surface, `Some(false)` strictly outside, `None`
/// unresolved. Surface membership is decided by the exact predicates, so a
/// point on a face counts as on the material — what a weld path lying exactly
/// on both parts of a tight joint needs (#724).
pub fn on_or_inside(
    mesh: &TriMesh,
    point: [f64; 3],
    limits: Limits,
) -> Result<Option<bool>, ContactError> {
    let shell = Shell::of(mesh)?;
    let mut preds = Predicates::new(limits.max_exact_predicates);
    match classify_point(&mut preds, &shell.tris, point)? {
        Side::Inside | Side::OnSurface => Ok(Some(true)),
        Side::Outside => Ok(Some(false)),
        Side::Unresolved => Ok(None),
    }
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
/// The samples are the vertices of `inside`; per triangle, the midpoint
/// between one corner and the midpoint of the opposite edge — a point strictly
/// inside the face by convexity with exactly representable coordinates, which
/// reaches the interior of aligned prisms whose vertices all lie on the
/// shell's walls; and, per surface edge of `inside` (an edge whose adjacent
/// triangles are not coplanar), the midpoint between two consecutive
/// transversal pierce points of the edge through `shell`'s triangles — the
/// point where the edge runs through the shell's interior, which reaches the
/// slivers no vertex or face probe lands in: a flush part
/// whose evaluated placement rounds one or two units in the last place into
/// the host dips by that ulp along its whole footprint, and the edge
/// midpoints are the samples that measure it (#721). A midpoint of a
/// non-convex traversal can sit outside the shell; the strict containment
/// classification filters it, so a wrong candidate never measures. The value
/// is a deterministic lower bound of the true penetration depth. At most
/// [`Limits::max_candidate_pairs`] contained samples are measured, in mesh
/// order, so a larger shell keeps the work bounded.
pub fn penetration_depth(
    inside: &TriMesh,
    shell: &TriMesh,
    limits: Limits,
) -> Result<Option<f64>, ContactError> {
    let shell = Shell::of(shell)?;
    let mut preds = Predicates::new(limits.max_exact_predicates);
    let mut deepest: Option<f64> = None;
    let mut measured = 0u64;
    let mut samples = samples_of(inside)?;
    samples.extend(edge_traversal_samples(inside, &shell, &mut preds, limits)?);
    for sample in &samples {
        if !shell.bounds.contains_point(*sample) {
            continue;
        }
        let contained = shell_contains(&mut preds, &shell, *sample)?;
        if contained != Some(true) {
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

/// The penetration depth of a strict surface crossing, measured from the
/// intersection itself: the largest inside run of a strictly crossing
/// surface edge through the other shell, in the meshes' coordinate units
/// (millimetres in the product), or `None` when no surface edge strictly
/// crosses the other shell or any run cannot be decided.
///
/// A surface edge is a mesh edge whose two adjacent triangles are not
/// coplanar, decided with the exact predicates; the edge set is a property
/// of the surface, so it survives a re-triangulation that keeps the
/// vertices, and the measured run never depends on where vertices or face
/// probes happen to lie. One edge's run is the total length of the intervals
/// between its consecutive strict crossings of the other shell whose
/// midpoint the exact predicates place strictly inside: between consecutive
/// transversal crossings the edge stays on one side of the surface, so the
/// midpoint decides the interval. A midpoint that lies on the surface or
/// stays unresolved refuses the whole measurement, and so does a crossing
/// whose pierce parameter is not representable over binary64: a flush wall
/// or a graze reports `None`, never a guessed value. At most
/// [`Limits::max_candidate_pairs`] crossing edges are measured, in mesh
/// order, so a larger shell keeps the work bounded.
pub fn crossing_depth(
    a: &TriMesh,
    b: &TriMesh,
    limits: Limits,
) -> Result<Option<f64>, ContactError> {
    let mut best: Option<f64> = None;
    let mut measured = 0u64;
    for (mesh, other) in [(a, b), (b, a)] {
        if measured >= limits.max_candidate_pairs {
            break;
        }
        let shell = Shell::of(other)?;
        let index = BoxIndex::build(&shell.boxes, limits.max_index_nodes, || false)
            .map_err(ContactError::Index)?;
        let mut preds = Predicates::new(limits.max_exact_predicates);
        for (s0, s1) in surface_edges(&mut preds, mesh)? {
            if measured >= limits.max_candidate_pairs {
                break;
            }
            match edge_inside_run(&mut preds, &shell, &index, s0, s1, limits, &mut measured)? {
                RunDecision::NoCrossing => {}
                RunDecision::Refused => return Ok(None),
                RunDecision::Run(run) => {
                    best = Some(match best {
                        Some(x) if x >= run => x,
                        _ => run,
                    });
                }
            }
        }
    }
    Ok(best)
}

/// The penetration depth of a coincident face (#741), in the meshes'
/// coordinate units (millimetres in the product), or `None` when no shared
/// face exists or no sample can be proven inside both shells.
///
/// A coincident face is a pair of meeting triangles, one of each shell, that
/// lie in one exact plane, overlap in area and face the same way: both
/// shells keep their material on the inward side of the shared area. Its
/// vertices and face probes all sit on the other surface, so the containment
/// samples of [`penetration_depth`] read nothing there — a duplicate part, or
/// a copy shifted by less than the contact tolerance, would measure only the
/// slivers. The samples here start on the shared face instead: from each face
/// probe of a triangle with a coincident partner, the segment along the
/// inward normal runs inside both shells until it first pierces either
/// surface; the midpoint of that first interval is a candidate, kept only
/// when the exact containment classification places it strictly inside
/// both shells. The value is the deepest kept candidate's distance to the
/// other shell's surface — the same reading as [`penetration_depth`], a
/// deterministic lower bound, never the length of the ray. At most
/// [`Limits::max_candidate_pairs`] partner tests and candidates are spent,
/// so a large shell keeps the work bounded; a probe whose pierce parameter
/// is not representable is skipped.
pub fn coincident_depth(
    a: &TriMesh,
    b: &TriMesh,
    limits: Limits,
) -> Result<Option<f64>, ContactError> {
    let shell_a = Shell::of(a)?;
    let shell_b = Shell::of(b)?;
    if !shell_a.bounds.overlaps(&shell_b.bounds) {
        return Ok(None);
    }
    let union = shell_a.bounds.union(&shell_b.bounds);
    let span = [
        union.max[0] - union.min[0],
        union.max[1] - union.min[1],
        union.max[2] - union.min[2],
    ];
    // Beyond the union box from any point inside it: the ray's far end lies
    // outside both shells.
    let reach = (span[0] * span[0] + span[1] * span[1] + span[2] * span[2]).sqrt() + 1.0;
    if !reach.is_finite() {
        return Err(ContactError::Range);
    }
    let index_a = BoxIndex::build(&shell_a.boxes, limits.max_index_nodes, || false)
        .map_err(ContactError::Index)?;
    let index_b = BoxIndex::build(&shell_b.boxes, limits.max_index_nodes, || false)
        .map_err(ContactError::Index)?;
    let cap = usize::try_from(limits.max_candidate_pairs).unwrap_or(0);
    let mut preds = Predicates::new(limits.max_exact_predicates);
    let mut deepest: Option<f64> = None;
    let mut spent = 0u64;
    let sides = [
        (&shell_a, &index_a, &shell_b, &index_b),
        (&shell_b, &index_b, &shell_a, &index_a),
    ];
    for (own, own_index, other, other_index) in sides {
        for (ti, tri) in own.tris.iter().enumerate() {
            let Some(tri_box) = own.boxes.get(ti) else {
                return Err(ContactError::Range);
            };
            let mut partnered = false;
            for oi in other_index
                .overlapping(tri_box, cap)
                .map_err(ContactError::Index)?
            {
                let Some(partner) = other.tris.get(usize::try_from(oi).unwrap_or(usize::MAX))
                else {
                    return Err(ContactError::Range);
                };
                spent += 1;
                if spent > limits.max_candidate_pairs {
                    return Ok(deepest);
                }
                if triangles_meet(&mut preds, tri, partner)?
                    && coplanar_orientation(&mut preds, tri, partner)? == Coplanar::Same
                {
                    partnered = true;
                    break;
                }
            }
            if !partnered {
                continue;
            }
            let Some(inward) = inward_unit_normal(tri) else {
                continue;
            };
            for probe in face_probes(tri[0], tri[1], tri[2]) {
                spent += 1;
                if spent > limits.max_candidate_pairs {
                    return Ok(deepest);
                }
                let far = [
                    probe[0] + inward[0] * reach,
                    probe[1] + inward[1] * reach,
                    probe[2] + inward[2] * reach,
                ];
                let Some(first) = first_pierce(
                    &mut preds,
                    probe,
                    far,
                    [(own, own_index), (other, other_index)],
                    cap,
                )?
                else {
                    continue;
                };
                let half = first * 0.5;
                let candidate = [
                    probe[0] + (far[0] - probe[0]) * half,
                    probe[1] + (far[1] - probe[1]) * half,
                    probe[2] + (far[2] - probe[2]) * half,
                ];
                if shell_contains(&mut preds, own, candidate)? != Some(true)
                    || shell_contains(&mut preds, other, candidate)? != Some(true)
                {
                    continue;
                }
                let depth = point_shell_closest(candidate, other).0;
                deepest = Some(match deepest {
                    Some(best) if best >= depth => best,
                    _ => depth,
                });
            }
        }
    }
    Ok(deepest)
}

/// The inward unit normal of a triangle: the opposite of its outward normal
/// `(b - a) x (c - a)`, measured in `f64`. `None` for a degenerate or
/// unrepresentable normal.
fn inward_unit_normal(tri: &Tri) -> Option<[f64; 3]> {
    let [a, b, c] = *tri;
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ];
    let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if !(length.is_finite() && length > 0.0) {
        return None;
    }
    Some([-n[0] / length, -n[1] / length, -n[2] / length])
}

/// The smallest pierce parameter of the segment `s0 s1` through the
/// triangles of the given shells, closed admit (a boundary pierce bounds the
/// interval too), or `None` when nothing is pierced or a pierce parameter is
/// not representable — the caller then skips the segment rather than guess.
/// A triangle whose plane contains `s0` is never pierced by the segment
/// (`segment_pierces_triangle` needs strictly opposite sides), so the
/// starting face and its coplanar partner drop out by construction.
fn first_pierce(
    preds: &mut Predicates,
    s0: [f64; 3],
    s1: [f64; 3],
    shells: [(&Shell, &BoxIndex); 2],
    cap: usize,
) -> Result<Option<f64>, ContactError> {
    let Some(query) = Aabb::from_points([s0, s1]) else {
        return Ok(None);
    };
    let mut first: Option<f64> = None;
    for (shell, index) in shells {
        for ci in index
            .overlapping(&query, cap)
            .map_err(ContactError::Index)?
        {
            let Some(tri) = shell.tris.get(usize::try_from(ci).unwrap_or(usize::MAX)) else {
                return Err(ContactError::Range);
            };
            if segment_pierces_triangle(preds, s0, s1, tri)? == Pierce::None {
                continue;
            }
            let Some(t) = crossing_parameter(s0, s1, tri) else {
                return Ok(None);
            };
            first = Some(match first {
                Some(best) if best <= t => best,
                _ => t,
            });
        }
    }
    Ok(first)
}

/// One measurement decision per surface edge.
#[derive(Debug, Clone, Copy, PartialEq)]
enum RunDecision {
    /// The edge has no strict crossing of the other shell.
    NoCrossing,
    /// A crossing's run cannot be decided: an interval midpoint lies on the
    /// surface or stays unresolved, or a pierce parameter is not
    /// representable.
    Refused,
    /// The measured inside length of the edge's crossing run.
    Run(f64),
}

/// Decides one surface edge against the other shell. Its strict crossings,
/// in order along the edge, bound the intervals the midpoints classify; a
/// proven crossing whose pierce parameter cannot be represented refuses.
fn edge_inside_run(
    preds: &mut Predicates,
    shell: &Shell,
    index: &BoxIndex,
    s0: [f64; 3],
    s1: [f64; 3],
    limits: Limits,
    measured: &mut u64,
) -> Result<RunDecision, ContactError> {
    let Some(edge_box) = Aabb::from_points([s0, s1]) else {
        return Ok(RunDecision::NoCrossing);
    };
    let cap = usize::try_from(limits.max_candidate_pairs).unwrap_or(0);
    let candidates = index
        .overlapping(&edge_box, cap)
        .map_err(ContactError::Index)?;
    let mut crossings: Vec<f64> = Vec::new();
    for ai in candidates {
        let Some(tri) = shell.tris.get(usize::try_from(ai).unwrap_or(usize::MAX)) else {
            return Err(ContactError::Range);
        };
        // The closed admit: a boundary pierce still bounds a crossing run's
        // intervals, and an undecided interval midpoint refuses downstream.
        if segment_pierces_triangle(preds, s0, s1, tri).map_err(ContactError::from)? == Pierce::None
        {
            continue;
        }
        let Some(t) = crossing_parameter(s0, s1, tri) else {
            return Ok(RunDecision::Refused);
        };
        crossings.push(t);
    }
    if crossings.is_empty() {
        return Ok(RunDecision::NoCrossing);
    }
    *measured += 1;
    crossings.sort_by(f64::total_cmp);
    let step = [s1[0] - s0[0], s1[1] - s0[1], s1[2] - s0[2]];
    let length = (step[0] * step[0] + step[1] * step[1] + step[2] * step[2]).sqrt();
    let mut inside = 0.0f64;
    let mut lo = 0.0f64;
    for hi in crossings.iter().copied().chain(std::iter::once(1.0)) {
        if hi > lo {
            let mid = lo + (hi - lo) * 0.5;
            let point = [
                s0[0] + step[0] * mid,
                s0[1] + step[1] * mid,
                s0[2] + step[2] * mid,
            ];
            match shell_contains(preds, shell, point)? {
                Some(true) => inside += hi - lo,
                Some(false) => {}
                // The interval meets a surface the crossings did not bound:
                // a flush run along a face or a graze. Refuse, never guess.
                None => return Ok(RunDecision::Refused),
            }
        }
        lo = hi;
    }
    Ok(RunDecision::Run(inside * length))
}

/// The pierce parameter of a proven strict crossing: the plane-side values
/// over binary64, with the exact side decision already taken by the
/// predicates. `None` when the ratio is not representable (the values
/// underflow or overflow against each other); the caller refuses the
/// measurement instead of guessing with a broken parameter.
fn crossing_parameter(s0: [f64; 3], s1: [f64; 3], tri: &Tri) -> Option<f64> {
    let [a, b, c] = *tri;
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ];
    let side = |p: [f64; 3]| {
        let d = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
        d[0] * n[0] + d[1] * n[1] + d[2] * n[2]
    };
    let (d0, d1) = (side(s0), side(s1));
    let denom = d0 - d1;
    if !denom.is_finite() || denom == 0.0 {
        return None;
    }
    let t = d0 / denom;
    if !t.is_finite() || t <= 0.0 || t >= 1.0 {
        return None;
    }
    Some(t)
}

/// One surface edge as its two endpoint positions.
type SurfaceEdge = ([f64; 3], [f64; 3]);

/// The surface edges of a mesh: each edge whose adjacent triangles are not
/// coplanar, decided with the exact predicates, as its two endpoint
/// positions. An edge with fewer or more than two adjacent triangles is
/// skipped: the contact facts assume closed, consistently oriented shells,
/// and a boundary or non-manifold edge star has no single surface side to
/// cross. The vertex-index key order keeps the scan deterministic.
fn surface_edges(preds: &mut Predicates, mesh: &TriMesh) -> Result<Vec<SurfaceEdge>, ContactError> {
    let vertices = mesh.vertices();
    let at = |i: u32| -> Result<[f64; 3], ContactError> {
        usize::try_from(i)
            .ok()
            .and_then(|i| vertices.get(i))
            .copied()
            .ok_or(ContactError::Range)
    };
    // One slot per adjacent triangle's opposite vertex; a third marks a
    // non-manifold fan, which no measurement reads.
    let mut sides: BTreeMap<[u32; 2], Vec<[f64; 3]>> = BTreeMap::new();
    for tri in mesh.triangles() {
        let t = [at(tri[0])?, at(tri[1])?, at(tri[2])?];
        for (p, q, r) in [
            (tri[0], tri[1], t[2]),
            (tri[1], tri[2], t[0]),
            (tri[2], tri[0], t[1]),
        ] {
            let key = if p < q { [p, q] } else { [q, p] };
            let slot = sides.entry(key).or_default();
            if slot.len() < 3 {
                slot.push(r);
            }
        }
    }
    let mut edges = Vec::new();
    for (key, opposite) in &sides {
        let (Some(first), Some(second)) = (opposite.first(), opposite.get(1)) else {
            continue;
        };
        if opposite.len() != 2 {
            continue;
        }
        let a = at(key[0])?;
        let b = at(key[1])?;
        if preds.orient3d(a, b, *first, *second)? != 0 {
            edges.push((a, b));
        }
    }
    Ok(edges)
}

/// The measured distance from a point to a mesh's triangles, in the meshes'
/// coordinate units (millimetres in the product). A measurement over the
/// given triangles, not a predicate: a faceted arc reads its chord deviation,
/// and the caller's tolerance band absorbs that bound.
pub fn point_gap(point: [f64; 3], mesh: &TriMesh) -> Result<f64, ContactError> {
    Ok(point_shell_closest(point, &Shell::of(mesh)?).0)
}

/// An upper bound on distance to the surface over a whole segment. Distance
/// to a closed triangle is convex: the larger endpoint distance bounds the
/// segment. Any triangle supplies an upper bound on distance to the mesh;
/// the smallest of those bounds is still conservative. Subdivision tightens it.
pub fn segment_gap_upper(
    from: [f64; 3],
    to: [f64; 3],
    mesh: &TriMesh,
) -> Result<f64, ContactError> {
    let shell = Shell::of(mesh)?;
    Ok(shell.tris.iter().fold(f64::INFINITY, |best, tri| {
        best.min(
            point_triangle_closest2(from, tri)
                .0
                .max(point_triangle_closest2(to, tri).0)
                .sqrt(),
        )
    }))
}

/// A representable interior subdivision point proven collinear with the
/// original segment. Rounding must not replace a segment with a kinked path;
/// an unrepresentable subdivision stays unresolved.
pub fn segment_midpoint(
    a: [f64; 3],
    b: [f64; 3],
    limits: Limits,
) -> Result<Option<[f64; 3]>, ContactError> {
    let mid = [
        a[0] * 0.5 + b[0] * 0.5,
        a[1] * 0.5 + b[1] * 0.5,
        a[2] * 0.5 + b[2] * 0.5,
    ];
    if mid == a || mid == b {
        return Ok(None);
    }
    let mut preds = Predicates::new(limits.max_exact_predicates);
    for (u, v) in [(0, 1), (0, 2), (1, 2)] {
        let component = |p: [f64; 3], i| p.get(i).copied().ok_or(ContactError::Range);
        if preds.orient2d(
            [component(a, u)?, component(a, v)?],
            [component(b, u)?, component(b, v)?],
            [component(mid, u)?, component(mid, v)?],
        )? != 0
        {
            return Ok(None);
        }
    }
    Ok(Some(mid))
}

// Exact path parameters: comparing cross-products avoids rounding a shared
// triangle edge's parameter (for example 1/7) differently on its two sides.
#[derive(Clone)]
struct SegmentFraction {
    num: Big,
    den: Big,
}

impl SegmentFraction {
    fn cmp(&self, other: &Self) -> Result<core::cmp::Ordering, ContactError> {
        let left = self.num.mul(&other.den).map_err(|_| ContactError::Range)?;
        let right = other.num.mul(&self.den).map_err(|_| ContactError::Range)?;
        Ok(left.cmp_mag(&right)) // numerators are non-negative, denominators positive
    }
}

fn triangle_segment_interval(
    a: [f64; 3],
    b: [f64; 3],
    tri: &Tri,
    preds: &mut Predicates,
    zero: &SegmentFraction,
    one: &SegmentFraction,
) -> Result<Option<(SegmentFraction, SegmentFraction)>, ContactError> {
    let [v0, v1, v2] = *tri;
    if preds.orient3d(v0, v1, v2, a)? != 0 || preds.orient3d(v0, v1, v2, b)? != 0 {
        return Ok(None);
    }
    let Some((axis, _)) = preds.normal_axis(v0, v1, v2)? else {
        return Err(ContactError::Range);
    };
    let exponent = min_exponent(tri.iter().chain([&a, &b]).copied().flatten());
    let xy = |p| -> Result<[Big; 2], ContactError> {
        let [x, y] = project(p, axis);
        Ok([
            scaled(x, exponent).map_err(|_| ContactError::Range)?,
            scaled(y, exponent).map_err(|_| ContactError::Range)?,
        ])
    };
    let (v0, v1, v2, a, b) = (xy(v0)?, xy(v1)?, xy(v2)?, xy(a)?, xy(b)?);
    let orient = |u: &[Big; 2], v: &[Big; 2], p: &[Big; 2]| -> Result<Big, ContactError> {
        let value = || -> Result<Big, crate::bigint::Overflow> {
            v[0].sub(&u[0])?
                .mul(&p[1].sub(&u[1])?)?
                .sub(&v[1].sub(&u[1])?.mul(&p[0].sub(&u[0])?)?)
        };
        value().map_err(|_| ContactError::Range)
    };
    let sign = orient(&v0, &v1, &v2)?.sign();
    let mut lo = zero.clone();
    let mut hi = one.clone();
    for (u, v) in [(&v0, &v1), (&v1, &v2), (&v2, &v0)] {
        let inward = |p| -> Result<Big, ContactError> {
            let value = orient(u, v, p)?;
            Ok(if sign < 0 { value.negated() } else { value })
        };
        let (d0, d1) = (inward(&a)?, inward(&b)?);
        if d0.sign() < 0 && d1.sign() < 0 {
            return Ok(None);
        }
        if d0.sign() >= 0 && d1.sign() >= 0 {
            continue;
        }
        let den = d0.sub(&d1).map_err(|_| ContactError::Range)?;
        let t = if den.sign() < 0 {
            SegmentFraction {
                num: d0.negated(),
                den: den.negated(),
            }
        } else {
            SegmentFraction {
                num: d0.clone(),
                den,
            }
        };
        if d0.sign() < 0 {
            if t.cmp(&lo)?.is_gt() {
                lo = t;
            }
        } else if t.cmp(&hi)?.is_lt() {
            hi = t;
        }
        if lo.cmp(&hi)?.is_gt() {
            return Ok(None);
        }
    }
    Ok(Some((lo, hi)))
}

/// Proves surface coverage of a whole segment by the union of its exact,
/// coplanar triangle intervals. Shared edge parameters are compared as integer
/// fractions, so a different valid triangulation cannot introduce numeric gaps.
/// A hole, unresolved predicates or exhausted work never proves contact.
pub fn segment_on_surface(
    from: [f64; 3],
    to: [f64; 3],
    mesh: &TriMesh,
    limits: Limits,
) -> Result<bool, ContactError> {
    let shell = Shell::of(mesh)?;
    let unit = Big::from_i64_shifted(1, 0).map_err(|_| ContactError::Range)?;
    let zero = SegmentFraction {
        num: Big::ZERO,
        den: unit.clone(),
    };
    let one = SegmentFraction {
        num: unit.clone(),
        den: unit,
    };
    let mut preds = Predicates::new(limits.max_exact_predicates);
    let mut intervals = Vec::new();
    let mut spent = 0u64;
    for tri in &shell.tris {
        spent += 1;
        if spent > limits.max_candidate_pairs.min(4096) {
            return Ok(false);
        }
        if let Some((lo, hi)) = triangle_segment_interval(from, to, tri, &mut preds, &zero, &one)? {
            if lo.cmp(&zero)?.is_eq() && hi.cmp(&one)?.is_eq() {
                return Ok(true);
            }
            intervals.push((lo, hi));
        }
    }
    let mut covered = zero;
    loop {
        let mut next = covered.clone();
        for (lo, hi) in &intervals {
            spent += 1;
            if spent > limits.max_candidate_pairs {
                return Ok(false);
            }
            if !lo.cmp(&covered)?.is_gt() && hi.cmp(&next)?.is_gt() {
                next = hi.clone();
            }
        }
        if !next.cmp(&covered)?.is_gt() {
            return Ok(false);
        }
        if !next.cmp(&one)?.is_lt() {
            return Ok(true);
        }
        covered = next;
    }
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

/// The edge traversal samples of `inside` against `shell`: per surface edge —
/// an edge whose two adjacent triangles are not coplanar, the same
/// triangulation-invariant set the crossing run reads — the midpoints of
/// consecutive transversal pierce points through `shell`'s closed triangles.
/// Between an entry and the next exit the edge runs through the shell's
/// interior, so the midpoint is a candidate strictly-inside point with exactly
/// representable coordinates — the sample that measures a last-place dip no
/// vertex or face probe lands in (#721). A non-convex traversal can put a
/// midpoint outside; the caller's strict containment classification filters
/// that.
fn edge_traversal_samples(
    inside: &TriMesh,
    shell: &Shell,
    preds: &mut Predicates,
    limits: Limits,
) -> Result<Vec<[f64; 3]>, ContactError> {
    let index = BoxIndex::build(&shell.boxes, limits.max_index_nodes, || false)
        .map_err(ContactError::Index)?;
    let mut samples = Vec::new();
    for (s0, s1) in surface_edges(preds, inside)? {
        let Some(query) = Aabb::from_points([s0, s1]) else {
            return Err(ContactError::Range);
        };
        let candidates = index
            .overlapping(
                &query,
                usize::try_from(limits.max_candidate_pairs).unwrap_or(0),
            )
            .map_err(ContactError::Index)?;
        let mut pierces: Vec<(f64, [f64; 3])> = Vec::new();
        for ai in candidates {
            let ti = usize::try_from(ai).unwrap_or(usize::MAX);
            let Some(tri) = shell.tris.get(ti) else {
                return Err(ContactError::Range);
            };
            let [a, b, c] = *tri;
            let d0 = preds.orient3d(a, b, c, s0).map_err(ContactError::from)?;
            let d1 = preds.orient3d(a, b, c, s1).map_err(ContactError::from)?;
            if d0 == 0 || d1 == 0 || d0 == d1 {
                continue;
            }
            let signs = [
                preds.orient3d(s0, s1, a, b).map_err(ContactError::from)?,
                preds.orient3d(s0, s1, b, c).map_err(ContactError::from)?,
                preds.orient3d(s0, s1, c, a).map_err(ContactError::from)?,
            ];
            if signs.contains(&1) && signs.contains(&-1) {
                continue;
            }
            // The exact predicates admit the crossing; the pierce parameter
            // itself comes from the plain determinants — a measurement for
            // the sample coordinate, whose ulp error the strict containment
            // classification then absorbs.
            let (e0, e1) = (orient3d_value(a, b, c, s0), orient3d_value(a, b, c, s1));
            if e0 == e1 {
                continue;
            }
            let w = e0 / (e0 - e1);
            pierces.push((
                w,
                [
                    s0[0] + w * (s1[0] - s0[0]),
                    s0[1] + w * (s1[1] - s0[1]),
                    s0[2] + w * (s1[2] - s0[2]),
                ],
            ));
        }
        pierces.sort_unstable_by(|p, q| p.0.total_cmp(&q.0));
        for pair in pierces.chunks(2) {
            if let [p, q] = pair {
                samples.push([
                    (p.1[0] + q.1[0]) / 2.0,
                    (p.1[1] + q.1[1]) / 2.0,
                    (p.1[2] + q.1[2]) / 2.0,
                ]);
            }
        }
    }
    let cap = usize::try_from(limits.max_candidate_pairs).unwrap_or(0);
    samples.truncate(cap);
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

    /// The `shell_box` over `lo` and `hi` with every quad's diagonal flipped
    /// the other way: a different, equally valid triangulation of the same
    /// surface (the shared diagonal-flip vocabulary).
    fn flipped_shell(lo: [f64; 3], hi: [f64; 3]) -> TriMesh {
        let v = |i: usize| {
            [
                if i & 1 == 1 { hi[0] } else { lo[0] },
                if i & 2 == 2 { hi[1] } else { lo[1] },
                if i & 4 == 4 { hi[2] } else { lo[2] },
            ]
        };
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
        TriMesh::from_buffers(&(0..8).map(v).collect::<Vec<_>>(), &tris).unwrap()
    }

    #[test]
    fn a_through_box_with_no_contained_vertex_measures_the_crossing_run() {
        // #722's adversarial case: a column straight through a plate, with
        // every vertex of each body strictly outside the other and no face
        // probe in the overlap. The crossing run is the plate thickness,
        // for the centred column and every off-centre placement of the
        // issue's matrix, below the plate and flush with its bottom face.
        let plate = shell_box([0.0, 0.0, 0.0], [1000.0, 500.0, 20.0]);
        for (lo, hi, start) in [
            ([400.0, 150.0, -50.0], [600.0, 350.0, 1000.0], "centred"),
            (
                [150.0, 350.0, -50.0],
                [250.0, 450.0, 1000.0],
                "100 at 200,400",
            ),
            (
                [650.0, 250.0, -50.0],
                [750.0, 350.0, 1000.0],
                "100 at 700,300",
            ),
            (
                [270.0, 270.0, -50.0],
                [330.0, 330.0, 1000.0],
                "60 at 300,300",
            ),
            (
                [337.0, 81.0, -50.0],
                [537.0, 281.0, 1000.0],
                "200 at 437,181",
            ),
            ([400.0, 150.0, 0.0], [600.0, 350.0, 1000.0], "flush bottom"),
        ] {
            let column = shell_box(lo, hi);
            let run = crossing_depth(&plate, &column, limits())
                .unwrap()
                .unwrap_or_default();
            assert!((run - 20.0).abs() <= 1e-9, "{start}: {run}");
        }
        // A different valid triangulation of the same plate measures the
        // same run: the depth never follows the triangulation.
        let flipped_plate = flipped_shell([0.0, 0.0, 0.0], [1000.0, 500.0, 20.0]);
        let column = shell_box([400.0, 150.0, -50.0], [600.0, 350.0, 1000.0]);
        let plain = crossing_depth(&plate, &column, limits()).unwrap();
        let flipped = crossing_depth(&flipped_plate, &column, limits()).unwrap();
        assert_eq!(flipped, plain);
    }

    #[test]
    fn a_flush_overlapping_pair_refuses_the_crossing_run() {
        // Half-overlapping boxes with shared wall planes: the crossing
        // edges run along the other body's walls, so an interval midpoint
        // lies on a surface and the measurement refuses instead of
        // guessing. The caller falls back to the sampled depth (#722).
        let a = shell_box([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);
        let b = shell_box([5.0, 0.0, 5.0], [15.0, 10.0, 15.0]);
        let report = contact(&a, &b, limits()).unwrap();
        assert!(matches!(
            report.relation,
            Relation::Overlapping {
                surface_crossing: true
            }
        ));
        assert_eq!(crossing_depth(&a, &b, limits()).unwrap(), None);
    }

    #[test]
    fn a_crossing_run_needs_a_strict_surface_edge() {
        // Without a crossing surface edge there is nothing to measure: two
        // separated cubes and a fully contained cube both report `None`,
        // and the sampled measure stays the caller's only depth there.
        let a = shell_box([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);
        let far = shell_box([20.0, 20.0, 20.0], [25.0, 25.0, 25.0]);
        assert_eq!(crossing_depth(&a, &far, limits()).unwrap(), None);
        let contained = shell_box([3.0, 3.0, 3.0], [6.0, 6.0, 6.0]);
        assert_eq!(crossing_depth(&a, &contained, limits()).unwrap(), None);
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
    fn segment_proofs_cover_triangles_but_never_bridge_a_hole() {
        let mesh = cube([0.0, 0.0, 0.0], 10.0);
        let (a, b) = ([1.0, 2.0, 10.0], [8.0, 2.0, 10.0]);
        // The cap diagonal is crossed at parameter 1/7, which no finite
        // dyadic subdivision reaches. Flipping that diagonal keeps contact.
        assert!(segment_on_surface(a, b, &mesh, limits()).unwrap());
        let flipped = flipped_shell_box([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);
        assert!(segment_on_surface(a, b, &flipped, limits()).unwrap());
        let upper = segment_gap_upper([1.0, 1.0, 12.0], [9.0, 1.0, 12.0], &mesh).unwrap();
        assert!((upper - 2.0).abs() < 1e-9);
        let mesh = holed_plate();
        let (a, b) = ([-80.0, 0.0, 20.0], [80.0, 0.0, 20.0]);
        assert_eq!(point_gap(a, &mesh).unwrap(), 0.0);
        assert_eq!(point_gap(b, &mesh).unwrap(), 0.0);
        assert!(!segment_on_surface(a, b, &mesh, limits()).unwrap());
        assert!(point_gap([0.0, 0.0, 20.0], &mesh).unwrap() > 20.0);
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

    /// A box shell whose every quad carries the diagonal flipped against
    /// [`shell_box`]: a different, equally valid triangulation of the same
    /// surface.
    fn flipped_shell_box(lo: [f64; 3], hi: [f64; 3]) -> TriMesh {
        let v = |i: usize| {
            [
                if i & 1 == 1 { hi[0] } else { lo[0] },
                if i & 2 == 2 { hi[1] } else { lo[1] },
                if i & 4 == 4 { hi[2] } else { lo[2] },
            ]
        };
        let tris: Vec<[u32; 3]> = [
            [0, 2, 1],
            [2, 3, 1], // face z=lo
            [4, 5, 6],
            [5, 7, 6], // face z=hi
            [0, 1, 4],
            [1, 5, 4], // face y=lo
            [2, 6, 3],
            [6, 7, 3], // face y=hi
            [0, 4, 2],
            [4, 6, 2], // face x=lo
            [1, 3, 5],
            [3, 7, 5], // face x=hi
        ]
        .into_iter()
        .collect();
        TriMesh::from_buffers(&(0..8).map(v).collect::<Vec<_>>(), &tris).unwrap()
    }

    #[test]
    fn flush_walls_overlapping_in_depth_read_touching_then_overlapping() {
        // #721: two bodies with the same footprint, one shifted up: their
        // walls are coplanar and every crossing line runs along a face rim,
        // so no pierce proves a strict crossing — the closed-triangle hit
        // used to claim one and mask the true fact. The surfaces meet
        // (Touching with coplanar wall contact), and the containment samples
        // then read mixed — part of the body strictly inside, part outside —
        // which promotes the pair to a proven interior overlap without any
        // surface crossing.
        let a = shell_box([0.0, 0.0, 0.0], [1000.0, 500.0, 10.0]);
        let b = shell_box([0.0, 0.0, 5.0], [1000.0, 500.0, 15.0]);
        let report = contact(&a, &b, limits()).unwrap();
        assert_eq!(
            report.relation,
            Relation::Overlapping {
                surface_crossing: false
            }
        );
        // The depth measurement still measures the 5 mm slab.
        let depth = penetration_depth(&a, &b, limits()).unwrap();
        assert!(
            matches!(depth, Some(d) if (d - 5.0).abs() <= 1e-9),
            "{depth:?}"
        );
    }

    #[test]
    fn a_last_place_dip_into_the_host_is_measured_by_the_edge_traversal() {
        // #721: the evaluated placement of the authored quarter-turn rounds
        // the standing plate's back bottom corner a couple of units in the
        // last place into the host. The crossing is real and the overlap
        // sliver is thinner than a coordinate step at that height: no vertex
        // or face probe of either body lands strictly inside, so the depth
        // read `None` and a correctly modelled base stayed yellow forever.
        // The edge traversal sample — the midpoint between the entry and the
        // exit pierce of the host's own edge through the standing plate's
        // walls — lands in the sliver and measures the dip.
        let base = shell_box([0.0, 0.0, 0.0], [1000.0, 500.0, 10.0]);
        let e = 1.7763568394002505e-15;
        let vertices: Vec<[f64; 3]> = vec![
            [150.0, -e, 9.999999999999998],
            [230.0, -e, 9.999999999999998],
            [150.0, 10.0, 10.0],
            [230.0, 10.0, 10.0],
            [150.0, -e, 109.99999999999999],
            [230.0, -e, 109.99999999999999],
            [150.0, 10.0, 110.00000000000001],
            [230.0, 10.0, 110.00000000000001],
        ];
        let tris: Vec<[u32; 3]> = [
            [0, 2, 3],
            [0, 3, 1], // the dipped bottom face
            [4, 5, 7],
            [4, 7, 6], // the top face
            [0, 1, 5],
            [0, 5, 4], // the back wall, one step beyond y = 0
            [2, 6, 7],
            [2, 7, 3], // the front wall at y = 10
            [0, 4, 6],
            [0, 6, 2], // x = 150
            [1, 3, 7],
            [1, 7, 5], // x = 230
        ]
        .into_iter()
        .collect();
        let standing = TriMesh::from_buffers(&vertices, &tris).unwrap();
        // The dip is real: a proven crossing of the surfaces.
        let report = contact(&base, &standing, limits()).unwrap();
        assert_eq!(
            report.relation,
            Relation::Overlapping {
                surface_crossing: true
            }
        );
        // The host's own edge traversal measures the sliver's depth: tiny,
        // far inside the contact tolerance band.
        let depth = penetration_depth(&base, &standing, limits()).unwrap();
        assert!(
            matches!(depth, Some(d) if d > 0.0 && d < 1e-12),
            "{depth:?}"
        );
    }

    #[test]
    fn a_box_standing_across_the_cap_diagonal_touches_like_off_it() {
        // #721: the standing box's footprint crosses the slab's top-cap
        // diagonal. The diagonal edge lies in the contact plane and pierces
        // the standing box's wall planes exactly at their bottom edges — a
        // graze the closed-triangle crossing read as penetration, so a
        // correctly modelled column base never reached green. The graze is a
        // touch: the standing box reports face contact, the same fact as the
        // identical box clear of the diagonal, under either valid
        // triangulation of the slab's caps.
        let slab = shell_box([0.0, 0.0, 0.0], [100.0, 100.0, 20.0]);
        let flipped = flipped_shell_box([0.0, 0.0, 0.0], [100.0, 100.0, 20.0]);
        // The first footprint crosses both caps' diagonals; the second
        // crosses neither.
        let across = shell_box([30.0, 50.0, 20.0], [60.0, 80.0, 40.0]);
        let clear = shell_box([30.0, 5.0, 20.0], [60.0, 25.0, 40.0]);
        let mut reports = Vec::new();
        for slab in [&slab, &flipped] {
            for standing in [&across, &clear] {
                let report = contact(slab, standing, limits()).unwrap();
                assert_eq!(report.relation, Relation::Touching);
                assert!(report.face_contact);
                reports.push(report);
            }
        }
        // Every placement and triangulation reads the same contact facts.
        for report in reports.iter().skip(1) {
            assert_eq!(report, &reports[0]);
        }
        // And the standing relation is decided from the surfaces alone: no
        // containment promotion had to run for it.
        assert_eq!(
            contact(&slab, &across, limits()).unwrap(),
            contact(&flipped, &clear, limits()).unwrap()
        );
    }

    #[test]
    fn a_shared_edge_pierce_into_the_interior_still_crosses_and_a_graze_does_not() {
        // #721: dropping the closed-triangle hit alone would miss the
        // segment that pierces the surface exactly through the edge two
        // triangles share. The shell's own continuation confirms such a
        // pierce as a crossing; a graze along a face plane or a rim touch
        // stays a touch.
        let mut preds = Predicates::new(limits().max_exact_predicates);
        // A quad split along its diagonal: the segment pierces it exactly
        // through the shared diagonal, from below into the interior.
        let quad = TriMesh::from_buffers(
            &[
                [0.0, 0.0, 20.0],
                [100.0, 0.0, 20.0],
                [0.0, 100.0, 20.0],
                [100.0, 100.0, 20.0],
            ],
            &[[0, 1, 3], [0, 3, 2]],
        )
        .unwrap();
        let shell = Shell::of(&quad).unwrap();
        let s0 = [50.0, 50.0, 10.0];
        let s1 = [50.0, 50.0, 30.0];
        for ti in 0..shell.tris.len() {
            assert!(
                pierce_decides_crossing(&mut preds, &shell, ti, s0, s1).unwrap(),
                "triangle {ti} must confirm the pierce through their shared edge"
            );
        }
        // The quad's own diagonal split decides through the continuation;
        // a pierce strictly inside one triangle decides on its own.
        let single = TriMesh::from_buffers(
            &[
                [0.0, 0.0, 20.0],
                [100.0, 0.0, 20.0],
                [0.0, 100.0, 20.0],
                [100.0, 100.0, 20.0],
            ],
            &[[0, 1, 3]],
        )
        .unwrap();
        let shell = Shell::of(&single).unwrap();
        assert!(
            pierce_decides_crossing(
                &mut preds,
                &shell,
                0,
                [60.0, 20.0, 10.0],
                [60.0, 20.0, 30.0]
            )
            .unwrap()
        );
        // A wall quad split along its diagonal into two coplanar triangles:
        // a segment running in the z=20 plane through their shared edge
        // passes through the wall's face — the continuation confirms it.
        // (The pierce lands in the upper triangle; the lower one misses it
        // across the diagonal.)
        let wall = TriMesh::from_buffers(
            &[
                [0.0, 0.0, 0.0],
                [100.0, 0.0, 0.0],
                [100.0, 0.0, 20.0],
                [0.0, 0.0, 20.0],
                [100.0, 0.0, 40.0],
                [0.0, 0.0, 40.0],
            ],
            &[[0, 1, 2], [0, 2, 3], [2, 5, 4], [2, 3, 5]],
        )
        .unwrap();
        let shell = Shell::of(&wall).unwrap();
        let g0 = [50.0, -100.0, 20.0];
        let g1 = [50.0, 100.0, 20.0];
        assert!(!pierce_decides_crossing(&mut preds, &shell, 0, g0, g1).unwrap());
        assert!(pierce_decides_crossing(&mut preds, &shell, 1, g0, g1).unwrap());
        // The wall as one quad ending at the rim z=20: the same graze
        // touches the surface's edge and crosses nothing.
        let rim = TriMesh::from_buffers(
            &[
                [0.0, 0.0, 0.0],
                [100.0, 0.0, 0.0],
                [100.0, 0.0, 20.0],
                [0.0, 0.0, 20.0],
            ],
            &[[0, 1, 2], [0, 2, 3]],
        )
        .unwrap();
        let shell = Shell::of(&rim).unwrap();
        assert!(!pierce_decides_crossing(&mut preds, &shell, 0, g0, g1).unwrap());
        assert!(!pierce_decides_crossing(&mut preds, &shell, 1, g0, g1).unwrap());
        // The #721 shape: the wall's bottom edge is shared with a
        // perpendicular bottom cap. A segment lying in the cap's plane
        // pierces the wall's plane exactly at that edge — the continuation
        // is not coplanar, so the segment runs along the cap's surface and
        // the pierce stays a touch, never a crossing.
        let capped = TriMesh::from_buffers(
            &[
                [0.0, 0.0, 20.0],
                [100.0, 0.0, 20.0],
                [100.0, 0.0, 40.0],
                [0.0, 0.0, 40.0],
                [100.0, 100.0, 20.0],
                [0.0, 100.0, 20.0],
            ],
            &[[0, 1, 2], [0, 2, 3], [0, 4, 1], [0, 5, 4]],
        )
        .unwrap();
        let shell = Shell::of(&capped).unwrap();
        // Against the wall triangle: unconfirmed by the cap, a touch.
        assert!(!pierce_decides_crossing(&mut preds, &shell, 0, g0, g1).unwrap());
        // Against the cap triangle: the segment lies in its plane, no pierce.
        assert!(!pierce_decides_crossing(&mut preds, &shell, 2, g0, g1).unwrap());
    }

    #[test]
    fn a_duplicate_slab_is_a_coincident_overlap_under_either_triangulation() {
        // #741: two identical slabs share every face with the same outward
        // orientation. No edge strictly crosses the other shell and every
        // sample lies on the other surface, yet the interiors coincide: the
        // coincident faces prove the overlap, under either valid
        // triangulation of the second slab.
        let lo = [0.0, 0.0, 0.0];
        let hi = [100.0, 100.0, 10.0];
        let a = shell_box(lo, hi);
        for b in [shell_box(lo, hi), flipped_shell_box(lo, hi)] {
            let report = contact(&a, &b, limits()).unwrap();
            assert!(report.coincident, "{report:?}");
            assert_eq!(
                report.relation,
                Relation::Overlapping {
                    surface_crossing: false
                }
            );
            assert!(!report.face_contact, "{report:?}");
            // The samples read nothing on a duplicate; the coincident face
            // measures the mid-thickness point of the shared caps: 5 mm.
            assert_eq!(penetration_depth(&a, &b, limits()).unwrap(), None);
            let depth = coincident_depth(&a, &b, limits()).unwrap().unwrap();
            assert!((depth - 5.0).abs() <= 1e-9, "{depth}");
        }
    }

    #[test]
    fn a_copy_shifted_below_the_tolerance_measures_its_overlap_not_the_sliver() {
        // A copy 0.2 mm off along x: no strict crossing, and the samples
        // reach only the 0.2 mm end slivers. The shared caps and walls are
        // coincident faces, and they measure the real overlap.
        let a = shell_box([0.0, 0.0, 0.0], [100.0, 100.0, 10.0]);
        let b = shell_box([0.2, 0.0, 0.0], [100.2, 100.0, 10.0]);
        let report = contact(&a, &b, limits()).unwrap();
        assert!(report.coincident, "{report:?}");
        assert!(matches!(report.relation, Relation::Overlapping { .. }));
        let sampled = penetration_depth(&a, &b, limits())
            .unwrap()
            .into_iter()
            .chain(penetration_depth(&b, &a, limits()).unwrap())
            .fold(0.0f64, f64::max);
        assert!(sampled <= 0.2 + 1e-9, "{sampled}");
        let depth = coincident_depth(&a, &b, limits()).unwrap().unwrap();
        assert!((depth - 5.0).abs() <= 1e-9, "{depth}");
    }

    #[test]
    fn a_thin_coincident_strip_never_reads_the_ray_length() {
        // Two slabs overlapping in a 0.2 mm strip: their caps and walls are
        // coincident only inside that strip. The probes of the shared
        // triangles fall outside the strip and are refused by the exact
        // containment test; the measure never reports the ray's length.
        let a = shell_box([0.0, 0.0, 0.0], [100.0, 100.0, 10.0]);
        let b = shell_box([99.8, 0.0, 0.0], [199.8, 100.0, 10.0]);
        let report = contact(&a, &b, limits()).unwrap();
        assert!(report.coincident, "{report:?}");
        let depth = coincident_depth(&a, &b, limits()).unwrap();
        assert!(depth.is_none_or(|d| d <= 0.2 + 1e-9), "{depth:?}");
    }

    #[test]
    fn opposite_faces_and_flush_fits_are_never_coincident() {
        // Side-by-side boxes share a wall with opposite orientations: face
        // contact, no coincident face.
        let a = shell_box([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);
        let side = shell_box([10.0, 0.0, 0.0], [20.0, 10.0, 10.0]);
        let report = contact(&a, &side, limits()).unwrap();
        assert_eq!(report.relation, Relation::Touching);
        assert!(report.face_contact && !report.coincident, "{report:?}");
        // A box standing on a larger one, flush with one of its walls: the
        // flush walls share only a line, the base faces oppose.
        let base = shell_box([0.0, 0.0, 0.0], [100.0, 100.0, 10.0]);
        let standing = shell_box([0.0, 40.0, 10.0], [20.0, 60.0, 50.0]);
        let report = contact(&base, &standing, limits()).unwrap();
        assert_eq!(report.relation, Relation::Touching);
        assert!(report.face_contact && !report.coincident, "{report:?}");
        // A plug flush in a through-hole: its walls oppose the hole walls,
        // and the holed caps hold no area over the hole.
        let plug = shell_box([-30.0, -30.0, 0.0], [30.0, 30.0, 20.0]);
        let report = contact(&holed_plate(), &plug, limits()).unwrap();
        assert_eq!(report.relation, Relation::Touching);
        assert!(report.face_contact && !report.coincident, "{report:?}");
    }

    #[test]
    fn on_or_inside_counts_the_surface_as_material() {
        let cube = shell_box([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);
        // Off the cube centre, whose six axis rays all graze a face diagonal
        // (the documented unresolved ceiling).
        assert_eq!(
            on_or_inside(&cube, [3.0, 4.0, 2.0], limits()),
            Ok(Some(true))
        );
        assert_eq!(
            on_or_inside(&cube, [5.0, 5.0, 10.0], limits()),
            Ok(Some(true))
        );
        assert_eq!(
            on_or_inside(&cube, [10.0, 10.0, 10.0], limits()),
            Ok(Some(true))
        );
        assert_eq!(
            on_or_inside(&cube, [5.0, 5.0, 10.1], limits()),
            Ok(Some(false))
        );
        // The strict variant keeps the surface out.
        assert_eq!(contains(&cube, [5.0, 5.0, 10.0], limits()), Ok(None));
    }
}
