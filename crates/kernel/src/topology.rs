// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Shell certification for arbitrary triangle meshes (G06).
//!
//! [`analyze`] classifies a [`TriMesh`] by exact combinatorics (sorted directed-edge
//! incidence, no welding or healing), exact geometric degeneracy, exact signed
//! volume per shell and a bounded exact self-intersection scan. The verdict is
//! [`ShellStatus::Certified`] only when every proof succeeded; any budget or
//! range limit yields [`ShellStatus::Uncertain`], never an exact status.
//!
//! References are public mathematics only: edge incidence of triangulated
//! surfaces and the Euler-Poincare formula `V - E + F = 2 - 2g` per closed
//! orientable component; the divergence theorem for the signed volume
//! `sum(v0 . (v1 x v2)) / 6`; and the fact that two closed triangles meet
//! exactly when an edge of one meets the other.

use crate::bigint::Big;
use crate::diagnostic::Stage;
use crate::enclosure::Enclosure;
use crate::exact::{PredError, Predicates, min_exponent, triangle_det};
use crate::intersect::{Side, Tri, classify_point as classify_side, overlap_beyond_shared};
use crate::spatial::{Aabb, BoxIndex, IndexError, MAX_INDEX_NODES};
use crate::trimesh::{MAX_SHELLS, TriMesh};
use crate::verdict::{Fault, MeshCode, MeshError, ShellStatus, Uncertainty};
use beamcraft_geom::Point3;
use beamcraft_geom::tolerance::Tolerance;

/// Explicit work caps of one analysis; exceeding any yields `Uncertain`, never an exact status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Overlapping triangle-box pairs examined by the self-intersection scan.
    pub max_candidate_pairs: u64,
    /// Nodes of the spatial index (clamped to [`MAX_INDEX_NODES`]).
    pub max_index_nodes: usize,
    /// Exact-integer predicate fallbacks over the whole analysis; a repeated exact
    /// question served from the predicate reuse table charges nothing.
    pub max_exact_predicates: u64,
}

impl Limits {
    /// Production caps: 2^20 pairs, the full node cap and 2^18 exact fallbacks.
    pub const DEFAULT: Self = Self {
        max_candidate_pairs: 1 << 20,
        max_index_nodes: MAX_INDEX_NODES,
        max_exact_predicates: 1 << 18,
    };
}

/// Counts of every structural defect found, independent of the final verdict.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Defects {
    /// Triangles with a repeated vertex index.
    pub collapsed_triangles: usize,
    /// Non-collapsed triangles with an edge shorter than `eps_geom` (strict).
    pub short_edge_triangles: usize,
    /// Triangles whose three corners are exactly collinear.
    pub zero_area_triangles: usize,
    /// Vertices referenced by no triangle.
    pub unused_vertices: usize,
    /// Undirected edges used by one triangle.
    pub boundary_edges: usize,
    /// Undirected edges used by three or more triangles.
    pub non_manifold_edges: usize,
    /// Interior edges traversed twice in the same direction.
    pub inconsistent_edges: usize,
    /// Triangles repeating an earlier triangle's vertex set.
    pub duplicate_triangles: usize,
    /// Vertices whose triangles do not form one fan.
    pub non_manifold_vertices: usize,
}

impl Defects {
    fn degenerate(&self) -> usize {
        self.collapsed_triangles
            + self.short_edge_triangles
            + self.zero_area_triangles
            + self.unused_vertices
    }
}

/// Orientation of a closed, consistently oriented shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// Positive signed volume: normals point out of the body.
    Outward,
    /// Negative signed volume.
    Inward,
    /// Exactly zero signed volume.
    Zero,
    /// Not established: the shell is not closed, manifold and consistently oriented.
    Unknown,
}

/// Facts about one connected shell (triangles joined through shared vertices).
#[derive(Debug, Clone, PartialEq)]
pub struct ShellReport {
    /// Triangles in the shell.
    pub triangles: usize,
    /// Distinct vertices used by the shell.
    pub vertices: usize,
    /// Distinct undirected edges of the shell.
    pub edges: usize,
    /// Euler characteristic `V - E + F`.
    pub euler_characteristic: i64,
    /// Genus when the shell is closed and orientable-manifold, else `None`.
    pub genus: Option<u32>,
    /// True when every edge has two opposite triangles and every vertex one fan.
    pub closed: bool,
    /// Sign of the exact signed volume, when established.
    pub orientation: Orientation,
    /// Enclosure of the signed volume in mm3, only for closed shells.
    pub signed_volume_mm3: Option<Enclosure>,
}

/// Sealed result of [`analyze`]; the certificate that gates every measurement.
#[derive(Debug, Clone)]
pub struct Analysis {
    status: ShellStatus,
    defects: Defects,
    shells: Vec<ShellReport>,
    triangle_shell: Vec<u8>,
    tolerance: Tolerance,
    bounds: Aabb,
    vertices: usize,
    triangles: usize,
    edges: usize,
    exact_predicates: u64,
    candidate_pairs: u64,
    fingerprint: u64,
}

impl Analysis {
    /// The verdict, by documented precedence (see [`ShellStatus`]).
    pub const fn status(&self) -> ShellStatus {
        self.status
    }
    /// True only for [`ShellStatus::Certified`].
    pub const fn is_certified(&self) -> bool {
        self.status.is_certified()
    }
    /// Every defect count, including those below the verdict's precedence.
    pub const fn defects(&self) -> &Defects {
        &self.defects
    }
    /// Connected shells in ascending order of their lowest triangle index.
    pub fn shells(&self) -> &[ShellReport] {
        &self.shells
    }
    /// Sum of shell Euler characteristics (`2k` for `k` closed genus-0 shells).
    pub fn euler_characteristic(&self) -> i64 {
        self.shells.iter().map(|s| s.euler_characteristic).sum()
    }
    /// Distinct undirected edges of the whole mesh.
    pub const fn edge_count(&self) -> usize {
        self.edges
    }
    /// Vertex count of the analyzed mesh.
    pub const fn vertex_count(&self) -> usize {
        self.vertices
    }
    /// Triangle count of the analyzed mesh.
    pub const fn triangle_count(&self) -> usize {
        self.triangles
    }
    /// F06 section 4 scale and geometric floor used for degeneracy decisions.
    pub const fn tolerance(&self) -> Tolerance {
        self.tolerance
    }
    /// Exact bounds of the vertices that triangles use.
    pub const fn bounds(&self) -> &Aabb {
        &self.bounds
    }
    /// Exact-integer predicate fallbacks used (diagnostic).
    pub const fn exact_predicates_used(&self) -> u64 {
        self.exact_predicates
    }
    /// Overlapping box pairs examined by the self-intersection scan (diagnostic).
    pub const fn candidate_pairs_examined(&self) -> u64 {
        self.candidate_pairs
    }
    /// Stable `GEO_` code of a refusal, `None` for a certified shell.
    pub const fn code(&self) -> Option<MeshCode> {
        self.status.code()
    }
    /// Shell index of each triangle, in triangle order.
    pub fn triangle_shells(&self) -> &[u8] {
        &self.triangle_shell
    }
    pub(crate) const fn fingerprint(&self) -> u64 {
        self.fingerprint
    }
}

struct Dsu {
    parent: Vec<u32>,
}

impl Dsu {
    fn new(n: usize) -> Self {
        let count = u32::try_from(n).unwrap_or(u32::MAX);
        Self {
            parent: (0..count).collect(),
        }
    }
    fn find(&mut self, start: u32) -> u32 {
        let mut x = start;
        loop {
            let p = self.parent.get(x as usize).copied().unwrap_or(x);
            if p == x {
                return x;
            }
            let grand = self.parent.get(p as usize).copied().unwrap_or(p);
            if let Some(slot) = self.parent.get_mut(x as usize) {
                *slot = grand;
            }
            x = grand;
        }
    }
    fn union(&mut self, a: u32, b: u32) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb
            && let Some(slot) = self.parent.get_mut(ra.max(rb) as usize)
        {
            *slot = ra.min(rb);
        }
    }
}

#[derive(Clone, Copy)]
struct EdgeRecord {
    key: u64,
    tri: u32,
    corner: u8,
    forward: bool,
}

#[derive(Default, Clone)]
struct ShellAcc {
    triangles: usize,
    vertices: usize,
    edges: usize,
    boundary: usize,
    non_manifold: usize,
    inconsistent: usize,
    pinched: usize,
}

fn poll(cancelled: &mut impl FnMut(Stage) -> bool, stage: Stage) -> Result<(), MeshError> {
    if cancelled(stage) {
        Err(MeshError::cancelled(stage))
    } else {
        Ok(())
    }
}

fn corner_id(tri: u32, k: usize) -> u32 {
    tri.saturating_mul(3)
        .saturating_add(u32::try_from(k).unwrap_or(0))
}

/// Analyzes a mesh with the production limits and no cancellation.
pub fn analyze(mesh: &TriMesh) -> Result<Analysis, MeshError> {
    analyze_with(mesh, Limits::DEFAULT, |_| false)
}

/// Analyzes a mesh with explicit limits and a host cancellation callback.
///
/// Cancellation polls, in order, each reporting the stage that would run next:
/// P0 `Validation` at entry; P1 `Build` before edge incidence; P2 `Checks`
/// before degeneracy and orientation; P3 `Checks` before the self-intersection
/// index (only for an otherwise clean closed outward mesh), then one poll per
/// build and about every 1024 units of scan work; one `Checks` poll per
/// shell-containment probe pair; and a final `Quantities` poll before the
/// result is sealed. The callback returns true to discard everything; no result
/// escapes a cancelled call and no poll follows sealing.
pub fn analyze_with(
    mesh: &TriMesh,
    limits: Limits,
    cancelled: impl FnMut(Stage) -> bool,
) -> Result<Analysis, MeshError> {
    analyze_impl(mesh, None, limits, cancelled)
}

/// Like [`analyze_with`], but uses the body's own F06 scale instead of deriving
/// one from the vertices.
///
/// A mesh with no body inputs gets the vertex-0 rule of [`TriMesh::tolerance`].
/// A caller that knows the body (a plate, a member) must pass the `Tolerance`
/// derived from those inputs: the plate rule excludes thickness from `D`, so a
/// plate at exactly its inclusive minimum thickness is sound under its own
/// tolerance but can fall below the generic mesh rule's slightly larger floor.
pub fn analyze_with_tolerance(
    mesh: &TriMesh,
    tolerance: Tolerance,
    limits: Limits,
    cancelled: impl FnMut(Stage) -> bool,
) -> Result<Analysis, MeshError> {
    analyze_impl(mesh, Some(tolerance), limits, cancelled)
}

fn analyze_impl(
    mesh: &TriMesh,
    body_tolerance: Option<Tolerance>,
    limits: Limits,
    mut cancelled: impl FnMut(Stage) -> bool,
) -> Result<Analysis, MeshError> {
    poll(&mut cancelled, Stage::Validation)?;
    let internal = || MeshError::numeric(Stage::Build);
    let tolerance = match body_tolerance {
        Some(t) => t,
        None => mesh.tolerance()?,
    };
    let (vertices, triangles) = (mesh.vertices(), mesh.triangles());
    let (nv, nt) = (vertices.len(), triangles.len());

    poll(&mut cancelled, Stage::Build)?;
    let mut used = vec![false; nv];
    for &i in triangles.iter().flatten() {
        *used.get_mut(i as usize).ok_or_else(internal)? = true;
    }
    let mut defects = Defects {
        unused_vertices: used.iter().filter(|u| !**u).count(),
        ..Defects::default()
    };
    let bounds = Aabb::from_points(
        vertices
            .iter()
            .zip(&used)
            .filter(|(_, u)| **u)
            .map(|(p, _)| *p),
    )
    .ok_or_else(internal)?;

    // Shells: triangles joined through shared vertex indices.
    let mut vertex_dsu = Dsu::new(nv);
    for &[a, b, c] in triangles {
        vertex_dsu.union(a, b);
        vertex_dsu.union(b, c);
    }
    let mut shell_of_root: Vec<Option<usize>> = vec![None; nv];
    let mut vertex_shell: Vec<Option<usize>> = vec![None; nv];
    let mut triangle_shell: Vec<u8> = Vec::with_capacity(nt);
    let mut shell_count = 0usize;
    for &[a, _, _] in triangles {
        let root = vertex_dsu.find(a) as usize;
        let slot = shell_of_root.get_mut(root).ok_or_else(internal)?;
        let id = match *slot {
            Some(id) => id,
            None => {
                if shell_count >= MAX_SHELLS {
                    return Err(MeshError::new(
                        MeshCode::TopologyLimit,
                        Fault::Shells,
                        Stage::Build,
                    ));
                }
                *slot = Some(shell_count);
                shell_count += 1;
                shell_count - 1
            }
        };
        triangle_shell.push(u8::try_from(id).map_err(|_| internal())?);
    }
    let mut acc = vec![ShellAcc::default(); shell_count];
    for (tri, &shell) in triangles.iter().zip(&triangle_shell) {
        let a = acc.get_mut(shell as usize).ok_or_else(internal)?;
        a.triangles += 1;
        for &v in tri {
            let slot = vertex_shell.get_mut(v as usize).ok_or_else(internal)?;
            if slot.is_none() {
                *slot = Some(shell as usize);
                a.vertices += 1;
            }
        }
    }

    // Directed-edge incidence by sorting; self-loop edges of collapsed triangles are skipped.
    let mut records: Vec<EdgeRecord> = Vec::with_capacity(3 * nt);
    for (t, tri) in triangles.iter().enumerate() {
        let t32 = u32::try_from(t).map_err(|_| internal())?;
        let [a, b, c] = *tri;
        if a == b || b == c || a == c {
            defects.collapsed_triangles += 1;
        }
        for (k, (u, v)) in [(a, b), (b, c), (c, a)].into_iter().enumerate() {
            if u == v {
                continue;
            }
            let (lo, hi) = (u.min(v), u.max(v));
            records.push(EdgeRecord {
                key: (u64::from(lo) << 32) | u64::from(hi),
                tri: t32,
                corner: u8::try_from(k).map_err(|_| internal())?,
                forward: u < v,
            });
        }
    }
    records.sort_unstable_by_key(|r| (r.key, r.tri, r.corner));
    let mut corner_dsu = Dsu::new(3 * nt);
    let mut edge_total = 0usize;
    for group in records.chunk_by(|x, y| x.key == y.key) {
        let first = group.first().ok_or_else(internal)?;
        let shell = *triangle_shell
            .get(first.tri as usize)
            .ok_or_else(internal)?;
        let a = acc.get_mut(shell as usize).ok_or_else(internal)?;
        a.edges += 1;
        edge_total += 1;
        match group {
            [_] => {
                a.boundary += 1;
                defects.boundary_edges += 1;
            }
            [x, y] if x.forward != y.forward => {}
            [_, _] => {
                a.inconsistent += 1;
                defects.inconsistent_edges += 1;
            }
            _ => {
                a.non_manifold += 1;
                defects.non_manifold_edges += 1;
            }
        }
        // Fan connectivity is undirected: every triangle on an edge joins the
        // corners at that edge's two endpoints, whatever its direction.
        let corners = |r: &EdgeRecord| {
            let k = usize::from(r.corner);
            let (start, end) = (corner_id(r.tri, k), corner_id(r.tri, (k + 1) % 3));
            if r.forward {
                (start, end)
            } else {
                (end, start)
            }
        };
        let (lo0, hi0) = corners(first);
        for r in group.iter().skip(1) {
            let (lo, hi) = corners(r);
            corner_dsu.union(lo0, lo);
            corner_dsu.union(hi0, hi);
        }
    }
    let mut first_fan: Vec<Option<u32>> = vec![None; nv];
    let mut pinched = vec![false; nv];
    for (t, tri) in triangles.iter().enumerate() {
        let t32 = u32::try_from(t).map_err(|_| internal())?;
        let [a, b, c] = *tri;
        if a == b || b == c || a == c {
            continue;
        }
        for (k, &v) in tri.iter().enumerate() {
            let root = corner_dsu.find(corner_id(t32, k));
            let slot = first_fan.get_mut(v as usize).ok_or_else(internal)?;
            match slot {
                None => *slot = Some(root),
                Some(r) if *r != root => {
                    let flag = pinched.get_mut(v as usize).ok_or_else(internal)?;
                    if !*flag {
                        *flag = true;
                        defects.non_manifold_vertices += 1;
                        let shell = vertex_shell.get(v as usize).copied().flatten();
                        acc.get_mut(shell.ok_or_else(internal)?)
                            .ok_or_else(internal)?
                            .pinched += 1;
                    }
                }
                Some(_) => {}
            }
        }
    }
    let mut keys: Vec<[u32; 3]> = triangles
        .iter()
        .map(|&[a, b, c]| {
            let mut k = [a, b, c];
            k.sort_unstable();
            k
        })
        .collect();
    keys.sort_unstable();
    defects.duplicate_triangles = keys.windows(2).filter(|w| w.first() == w.get(1)).count();

    let mut shells: Vec<ShellReport> = Vec::with_capacity(shell_count);
    for a in &acc {
        let closed =
            a.boundary == 0 && a.non_manifold == 0 && a.inconsistent == 0 && a.pinched == 0;
        let chi = i64::try_from(a.vertices).unwrap_or(i64::MAX)
            - i64::try_from(a.edges).unwrap_or(i64::MAX)
            + i64::try_from(a.triangles).unwrap_or(i64::MAX);
        let genus = if closed && chi <= 2 && chi % 2 == 0 {
            u32::try_from((2 - chi) / 2).ok()
        } else {
            None
        };
        shells.push(ShellReport {
            triangles: a.triangles,
            vertices: a.vertices,
            edges: a.edges,
            euler_characteristic: chi,
            genus,
            closed,
            orientation: Orientation::Unknown,
            signed_volume_mm3: None,
        });
    }

    let mut preds = Predicates::new(limits.max_exact_predicates);
    let mut candidate_pairs = 0u64;
    let mut tri_points: Vec<Tri> = Vec::with_capacity(nt);
    for t in 0..nt {
        tri_points.push(mesh.triangle_points(t).ok_or_else(internal)?);
    }

    poll(&mut cancelled, Stage::Checks)?;
    // Geometric degeneracy: strict eps_geom coincidence, then exact collinearity.
    let mut degeneracy_uncertain = None;
    let eps = tolerance.eps_geom_mm();
    for (tri, pts) in triangles.iter().zip(&tri_points) {
        let [a, b, c] = *tri;
        if a == b || b == c || a == c {
            continue;
        }
        let [pa, pb, pc] = *pts;
        let point = |[x, y, z]: [f64; 3]| Point3::new(x, y, z);
        let mut short = false;
        for (p, q) in [(pa, pb), (pb, pc), (pc, pa)] {
            let d = point(p)
                .checked_distance_to(point(q))
                .map_err(|_| MeshError::numeric(Stage::Checks))?;
            short |= d < eps;
        }
        if short {
            defects.short_edge_triangles += 1;
            continue;
        }
        match preds.normal_axis(pa, pb, pc) {
            Ok(None) => defects.zero_area_triangles += 1,
            Ok(Some(_)) => {}
            Err(PredError::Budget) => degeneracy_uncertain = Some(Uncertainty::PredicateBudget),
            Err(PredError::Range) => degeneracy_uncertain = Some(Uncertainty::NumericRange),
        }
    }

    let mut status = if defects.degenerate() > 0 {
        Some(ShellStatus::Degenerate)
    } else if let Some(u) = degeneracy_uncertain {
        Some(ShellStatus::Uncertain(u))
    } else if defects.non_manifold_edges > 0
        || defects.duplicate_triangles > 0
        || defects.non_manifold_vertices > 0
        || shells.iter().any(|s| s.closed && s.genus.is_none())
    {
        Some(ShellStatus::NonManifold)
    } else if defects.boundary_edges > 0 {
        Some(ShellStatus::Open)
    } else if defects.inconsistent_edges > 0 {
        Some(ShellStatus::InconsistentOrientation)
    } else {
        None
    };

    // Exact signed volume per shell, only when every shell is closed and oriented.
    if status.is_none() {
        let exponent = min_exponent(vertices.iter().flatten().copied());
        let mut sums: Vec<Big> = vec![Big::ZERO; shell_count];
        for (pts, &shell) in tri_points.iter().zip(&triangle_shell) {
            let [a, b, c] = *pts;
            let det =
                triangle_det(exponent, a, b, c).map_err(|_| MeshError::numeric(Stage::Checks))?;
            let slot = sums.get_mut(shell as usize).ok_or_else(internal)?;
            *slot = slot
                .add(&det)
                .map_err(|_| MeshError::numeric(Stage::Checks))?;
        }
        let mut refusal = None;
        for (report, sum) in shells.iter_mut().zip(&sums) {
            report.orientation = match sum.sign() {
                1 => Orientation::Outward,
                -1 => Orientation::Inward,
                _ => Orientation::Zero,
            };
            let enclosure = sum
                .enclose(3 * exponent)
                .and_then(|e| e.div(Enclosure::point(6.0)));
            match enclosure {
                Some(v) => report.signed_volume_mm3 = Some(v),
                None => {
                    refusal = refusal.or(Some(ShellStatus::Uncertain(Uncertainty::NumericRange)))
                }
            }
        }
        if shells.iter().any(|s| s.orientation == Orientation::Inward) {
            status = Some(ShellStatus::Inward);
        } else if shells.iter().any(|s| s.orientation == Orientation::Zero) {
            status = Some(ShellStatus::Degenerate);
        } else {
            status = refusal;
        }
    }

    // Exact self-intersection scan over a bounded box-tree.
    if status.is_none() {
        poll(&mut cancelled, Stage::Checks)?;
        let boxes: Vec<Aabb> = tri_points
            .iter()
            .map(|t| Aabb::from_points(t.iter().copied()))
            .collect::<Option<_>>()
            .ok_or_else(internal)?;
        match BoxIndex::build(&boxes, limits.max_index_nodes, || cancelled(Stage::Checks)) {
            Err(IndexError::Cancelled) => return Err(MeshError::cancelled(Stage::Checks)),
            Err(_) => status = Some(ShellStatus::Uncertain(Uncertainty::IndexBudget)),
            Ok(index) => {
                let mut overlap = false;
                let mut uncertain = None;
                let result = index.for_each_overlapping_pair(
                    limits.max_candidate_pairs,
                    || cancelled(Stage::Checks),
                    |i, j| {
                        candidate_pairs += 1;
                        let pick = |t: u32| {
                            let t = t as usize;
                            Some((*tri_points.get(t)?, *triangles.get(t)?))
                        };
                        let (Some((ta, ia)), Some((tb, ib))) = (pick(i), pick(j)) else {
                            uncertain = Some(Uncertainty::NumericRange);
                            return Err(IndexError::Halted);
                        };
                        match overlap_beyond_shared(&mut preds, &ta, ia, &tb, ib) {
                            Ok(false) => Ok(()),
                            Ok(true) => {
                                overlap = true;
                                Err(IndexError::Halted)
                            }
                            Err(PredError::Budget) => {
                                uncertain = Some(Uncertainty::PredicateBudget);
                                Err(IndexError::Halted)
                            }
                            Err(PredError::Range) => {
                                uncertain = Some(Uncertainty::NumericRange);
                                Err(IndexError::Halted)
                            }
                        }
                    },
                );
                match result {
                    Err(IndexError::Cancelled) => return Err(MeshError::cancelled(Stage::Checks)),
                    Err(IndexError::PairCap) => {
                        status = Some(ShellStatus::Uncertain(Uncertainty::PairBudget));
                    }
                    Err(IndexError::Halted) => {
                        status = if overlap {
                            Some(ShellStatus::Overlapping)
                        } else {
                            uncertain.map(ShellStatus::Uncertain)
                        };
                    }
                    Err(_) => status = Some(ShellStatus::Uncertain(Uncertainty::IndexBudget)),
                    Ok(()) => {}
                }
            }
        }
    }

    // One shell strictly inside another: disjoint, so one vertex decides.
    if status.is_none() && shell_count > 1 {
        status = nested_shells(
            &mut cancelled,
            &mut preds,
            triangles,
            &tri_points,
            &triangle_shell,
            shell_count,
        )?;
    }

    poll(&mut cancelled, Stage::Quantities)?;
    Ok(Analysis {
        status: status.unwrap_or(ShellStatus::Certified),
        defects,
        shells,
        triangle_shell,
        tolerance,
        bounds,
        vertices: nv,
        triangles: nt,
        edges: edge_total,
        exact_predicates: preds.exact_calls(),
        candidate_pairs,
        fingerprint: mesh.fingerprint(),
    })
}

/// Detects any shell that lies inside another; `Ok(None)` means none does.
fn nested_shells(
    cancelled: &mut impl FnMut(Stage) -> bool,
    preds: &mut Predicates,
    triangles: &[[u32; 3]],
    tri_points: &[Tri],
    triangle_shell: &[u8],
    shell_count: usize,
) -> Result<Option<ShellStatus>, MeshError> {
    let internal = || MeshError::numeric(Stage::Checks);
    let mut per_shell: Vec<Vec<Tri>> = vec![Vec::new(); shell_count];
    let mut boxes: Vec<Option<Aabb>> = vec![None; shell_count];
    let mut probes: Vec<Vec<[f64; 3]>> = vec![Vec::new(); shell_count];
    let mut probe_ids: Vec<Vec<u32>> = vec![Vec::new(); shell_count];
    for ((pts, tri), &shell) in tri_points.iter().zip(triangles).zip(triangle_shell) {
        let s = shell as usize;
        per_shell.get_mut(s).ok_or_else(internal)?.push(*pts);
        let b = Aabb::from_points(pts.iter().copied()).ok_or_else(internal)?;
        let slot = boxes.get_mut(s).ok_or_else(internal)?;
        *slot = Some(slot.map_or(b, |old| old.union(&b)));
        for (corner, &v) in tri.iter().enumerate() {
            let ids = probe_ids.get_mut(s).ok_or_else(internal)?;
            if ids.len() < 8 && !ids.contains(&v) {
                ids.push(v);
                probes
                    .get_mut(s)
                    .ok_or_else(internal)?
                    .push(*pts.get(corner).ok_or_else(internal)?);
            }
        }
    }
    let mut unresolved = false;
    for outer in 0..shell_count {
        for inner in 0..shell_count {
            if outer == inner {
                continue;
            }
            let (Some(Some(ob)), Some(Some(ib))) = (boxes.get(outer), boxes.get(inner)) else {
                return Err(internal());
            };
            if !ob.contains_box(ib) {
                continue;
            }
            poll(cancelled, Stage::Checks)?;
            let outer_tris = per_shell.get(outer).ok_or_else(internal)?;
            let mut decided = false;
            for &q in probes.get(inner).ok_or_else(internal)? {
                match classify_side(preds, outer_tris, q) {
                    Ok(Side::Inside | Side::OnSurface) => {
                        return Ok(Some(ShellStatus::Overlapping));
                    }
                    Ok(Side::Outside) => {
                        decided = true;
                        break;
                    }
                    Ok(Side::Unresolved) => {}
                    Err(PredError::Budget) => {
                        return Ok(Some(ShellStatus::Uncertain(Uncertainty::PredicateBudget)));
                    }
                    Err(PredError::Range) => {
                        return Ok(Some(ShellStatus::Uncertain(Uncertainty::NumericRange)));
                    }
                }
            }
            unresolved |= !decided;
        }
    }
    Ok(unresolved.then_some(ShellStatus::Uncertain(Uncertainty::Containment)))
}

/// Where a point lies relative to a certified mesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointClass {
    /// Strictly inside the solid.
    Inside,
    /// Strictly outside every shell (including inside a tunnel).
    Outside,
    /// Exactly on a triangle, edge or vertex.
    OnSurface,
    /// Not decided; never a guess.
    Uncertain,
}

/// Classifies a point against a mesh whose [`Analysis`] is certified.
///
/// Surface membership is exact. Otherwise the first axis ray that avoids every
/// edge and vertex decides by crossing parity; if all six rays graze one, or
/// the predicate budget is exhausted, the result is `Uncertain`.
pub fn classify_point(
    mesh: &TriMesh,
    analysis: &Analysis,
    point: [f64; 3],
) -> Result<PointClass, MeshError> {
    if !analysis.is_certified() || analysis.fingerprint() != mesh.fingerprint() {
        return Err(MeshError::new(
            MeshCode::NotCertified,
            Fault::Analysis,
            Stage::Validation,
        ));
    }
    if point.iter().any(|v| !v.is_finite()) {
        return Err(MeshError::new(
            MeshCode::NonFinite,
            Fault::Coordinates,
            Stage::Validation,
        ));
    }
    let internal = || MeshError::numeric(Stage::Checks);
    let mut preds = Predicates::new(Limits::DEFAULT.max_exact_predicates);
    let mut per_shell: Vec<Vec<Tri>> = vec![Vec::new(); analysis.shells().len()];
    for (t, &shell) in analysis.triangle_shells().iter().enumerate() {
        per_shell
            .get_mut(shell as usize)
            .ok_or_else(internal)?
            .push(mesh.triangle_points(t).ok_or_else(internal)?);
    }
    let mut result = PointClass::Outside;
    for tris in &per_shell {
        match classify_side(&mut preds, tris, point) {
            Ok(Side::OnSurface) => return Ok(PointClass::OnSurface),
            Ok(Side::Inside) => return Ok(PointClass::Inside),
            Ok(Side::Outside) => {}
            Ok(Side::Unresolved) | Err(_) => result = PointClass::Uncertain,
        }
    }
    Ok(result)
}
