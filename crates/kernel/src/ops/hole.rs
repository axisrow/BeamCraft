// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Pilot through hole: subtract a straight prism that fully traverses a Z-prism body.
//!
//! The body must be a straight prism: its lowest and highest vertices define two heights
//! `z0 < z1`, every triangle at `z0` or at `z1` is a horizontal cap (one face each, facing
//! outward), and every other triangle is exactly vertical (exact zero projected area), which
//! allows extra vertices at intermediate heights on the walls (a vertical plane cut creates
//! them). The cutter is a G05 cavity (round, oval or polygonal; clockwise in XY) with a
//! `z` range that reaches past both caps by at least the body's geometric floor. The hole
//! must lie strictly inside the cap outline and clear of existing holes: this is decided by the
//! G05 profile admission of the new cap (exact signs, floors at `eps_geom`), so a hole that
//! touches or crosses the outline, touches another hole, lies inside one or encloses one is
//! refused. One cutter per call; further holes are further calls on the result body, and an
//! overlapping second cutter is refused like any other contact.
//!
//! Construction: both caps are re-triangulated with the cavity added as one more hole (the
//! cap loops of the body are kept vertex for vertex, so the untouched walls stay valid), the
//! cutter's own walls are added over `[z0, z1]`, and the candidate is certified. Genus grows
//! by exactly one, and the volume must equal the body volume minus the exact prism volume
//! `area * thickness`, enclosed outward.
//!
//! Local feature ids: the walls are `cut:f.h<ordinal>.s<k>` (edge `k` of a polygonal cavity)
//! or `cut:f.h<ordinal>` (the one curved wall of an approximated curve cavity, as G05's
//! `f.c<j>`); existing faces keep their ids. Provisional, like the plane-cut cap id.

use super::body::{Body, MAX_ORDINAL};
use super::diagnostic::{OpCode, OpDefect, OpFailure, OpField};
use super::seal::{Candidate, Expect, OpKind, OpResult, seal};
use super::section::{
    CapLoop, ChainError, area_sign, canonical_loop, chain_loops, split_loops, triangulate_cap,
};
use super::{ExactBudget, OpLimits, OpStats, poll};
use crate::bigint::Big;
use crate::diagnostic::Stage;
use crate::enclosure::Enclosure;
use crate::exact::{min_exponent, scaled};
use crate::profile::{Cavity, MAX_CONTOUR_POINTS};
use crate::trimesh::MAX_COORDINATE_MM;

/// A straight cutter: a clockwise cavity polygon in the body's XY plane swept over
/// `[z_from_mm, z_to_mm]` along Z.
#[derive(Debug, Clone, PartialEq)]
pub struct HoleCutter {
    cavity: Cavity,
    z_from_mm: f64,
    z_to_mm: f64,
    ordinal: u16,
}

impl HoleCutter {
    /// Creates a cutter without validating it; [`through_hole`] validates everything.
    ///
    /// `ordinal` (at most 999) names the hole walls; the caller (the future A04 command) owns
    /// the mapping from its authored hole to the ordinal and must keep it stable.
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
pub fn through_hole(body: &Body, cutter: &HoleCutter) -> Result<OpResult, OpFailure> {
    through_hole_with(body, cutter, &OpLimits::DEFAULT, |_| false)
}

/// Subtracts with explicit bounds and the F06 cancellation points: `Validation` at entry,
/// `Build` before the body analysis, `Checks` before the candidate's certification (whose
/// own G06 polls follow) and `Quantities` before sealing. A cancelled call returns no result
/// and nothing polls after sealing.
pub fn through_hole_with(
    body: &Body,
    cutter: &HoleCutter,
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
    let wall_id = |k: Option<usize>| match k {
        None => format!("cut:f.h{}", cutter.ordinal),
        Some(k) => format!("cut:f.h{}.s{k}", cutter.ordinal),
    };
    let stem = wall_id(None);
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
    let prism = analyze_prism(body, limits, &mut budget)?;

    let eps = body.tolerance().eps_geom_mm();
    let (below, above) = (prism.z0 - cutter.z_from_mm, cutter.z_to_mm - prism.z1);
    let range_refusal = if below < 0.0 || above < 0.0 {
        Some(OpDefect::CutterNotThrough)
    } else if below == 0.0 || above == 0.0 {
        Some(OpDefect::CutterTangent)
    } else if below < eps || above < eps {
        Some(OpDefect::CutterMargin)
    } else {
        None
    };
    if let Some(defect) = range_refusal {
        return Err(refuse(OpCode::OpUnsupported, defect, Stage::Ranges));
    }

    // New vertices: the cavity at the bottom and at the top height.
    let base = u32::try_from(body.mesh().vertices().len()).map_err(|_| internal())?;
    let m32 = u32::try_from(m).map_err(|_| internal())?;
    let mut vertices: Vec<[f64; 3]> = body.mesh().vertices().to_vec();
    vertices.extend(points.iter().map(|p| [p.x, p.y, prism.z0]));
    vertices.extend(points.iter().map(|p| [p.x, p.y, prism.z1]));
    let new_ring = |z: f64, first: u32| CapLoop {
        ids: (0..m32).map(|k| first + k).collect(),
        xyz: points.iter().map(|p| [p.x, p.y, z]).collect(),
    };
    let flat = |p: [f64; 3]| [p[0], p[1]];

    let mut top_holes = prism.top.1.clone();
    top_holes.push(new_ring(prism.z1, base + m32));
    let mut bottom_holes = prism.bottom.1.clone();
    bottom_holes.push(new_ring(prism.z0, base));
    let top_cap = triangulate_cap(
        &prism.top.0,
        &top_holes,
        flat,
        limits,
        body.tolerance(),
        OpField::Cap,
        false,
    )
    .map_err(|e| cutter_field(e, prism.top.1.len()))?;
    let bottom_cap = triangulate_cap(
        &prism.bottom.0,
        &bottom_holes,
        flat,
        limits,
        body.tolerance(),
        OpField::Cap,
        false,
    )
    .map_err(|e| cutter_field(e, prism.bottom.1.len()))?;

    let mut triangles = prism.walls;
    let mut faces = prism.wall_faces;
    for [a, b, c] in top_cap {
        triangles.push([a, b, c]);
        faces.push(prism.top_face);
    }
    for [a, b, c] in bottom_cap {
        triangles.push([a, c, b]);
        faces.push(prism.bottom_face);
    }

    let curved = cutter.cavity.approximation().is_some();
    let mut face_ids = body.face_table().to_vec();
    let mut normals = body.face_normals().to_vec();
    let first_face = face_ids.len();
    if curved {
        face_ids.push(wall_id(None));
        normals.push(None);
    }
    for k in 0..m {
        let j = (k + 1) % m;
        let (p, q) = (
            points.get(k).copied().ok_or_else(internal)?,
            points.get(j).copied().ok_or_else(internal)?,
        );
        let face = if curved {
            first_face
        } else {
            face_ids.push(wall_id(Some(k)));
            let (dx, dy) = (q.x - p.x, q.y - p.y);
            let length = (dx * dx + dy * dy).sqrt();
            if !(length > 0.0 && length.is_finite()) {
                return Err(internal());
            }
            let n = [dy / length, -dx / length, 0.0];
            normals.push(Some(n.map(|v| if v == 0.0 { 0.0 } else { v })));
            face_ids.len() - 1
        };
        let face = u16::try_from(face).map_err(|_| internal())?;
        let k32 = u32::try_from(k).map_err(|_| internal())?;
        let j32 = u32::try_from(j).map_err(|_| internal())?;
        let (bk, bj, tk, tj) = (base + k32, base + j32, base + m32 + k32, base + m32 + j32);
        triangles.push([bk, bj, tj]);
        triangles.push([bk, tj, tk]);
        faces.push(face);
        faces.push(face);
    }

    let removed = prism_volume(&cutter.cavity, prism.z0, prism.z1)?;
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
        new_vertices: 2 * m,
        loops: 2 + prism.top.1.len(),
        cap_points: prism.top.0.ids.len()
            + prism.top.1.iter().map(|h| h.ids.len()).sum::<usize>()
            + m,
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
            kind: OpKind::ThroughHole,
            euler: Some(shell.euler_characteristic - 2),
            genus: shell.genus.and_then(|g| g.checked_add(1)),
            analytic_volume: Some(analytic),
            plane_deviation_mm: None,
            stats,
        },
        &mut cancelled,
    )
}

// A profile diagnostic about the last hole of the cap is about the new cutter.
/// Shared with the steel crate.
pub fn cutter_field(mut error: OpFailure, existing_holes: usize) -> OpFailure {
    if error.field == OpField::Cap
        && error
            .profile
            .is_some_and(|d| usize::try_from(d.cavity.unwrap_or(u32::MAX)) == Ok(existing_holes))
    {
        error.field = OpField::Cutter;
    }
    error
}

/// Exact prism volume `|area| * (z1 - z0)` of the cavity, enclosed outward.
fn prism_volume(cavity: &Cavity, z0: f64, z1: f64) -> Result<Enclosure, OpFailure> {
    let range = || OpFailure::numeric(OpField::Volume, OpDefect::NumericRange, Stage::Quantities);
    let points = cavity.points();
    let e = min_exponent(points.iter().flat_map(|p| [p.x, p.y]).chain([z0, z1]));
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
    let thickness = s(z1)?.sub(&s(z0)?).map_err(|_| range())?;
    let doubled = area2.mul(&thickness).map_err(|_| range())?;
    let pow2 = e
        .checked_mul(3)
        .and_then(|v| v.checked_sub(1))
        .ok_or_else(range)?;
    doubled.enclose_clamped(pow2).ok_or_else(range)
}

/// The structure of a Z-prism body: heights, cap faces, cap loops and the untouched walls.
struct Prism {
    z0: f64,
    z1: f64,
    top_face: u16,
    bottom_face: u16,
    top: (CapLoop, Vec<CapLoop>),
    bottom: (CapLoop, Vec<CapLoop>),
    walls: Vec<[u32; 3]>,
    wall_faces: Vec<u16>,
}

fn analyze_prism(
    body: &Body,
    limits: &OpLimits,
    budget: &mut ExactBudget,
) -> Result<Prism, OpFailure> {
    let not_prism = |defect| OpFailure::unsupported(OpField::Body, defect, Stage::Build);
    let mesh = body.mesh();
    let vertices = mesh.vertices();
    let z0 = vertices.iter().map(|p| p[2]).fold(f64::INFINITY, f64::min);
    let z1 = vertices
        .iter()
        .map(|p| p[2])
        .fold(f64::NEG_INFINITY, f64::max);
    // Vertices at intermediate heights are allowed (a vertical cut crosses wall diagonals);
    // they can only belong to exactly vertical triangles, which is checked per triangle.
    if z0 >= z1 {
        return Err(not_prism(OpDefect::BodyNotPrism));
    }
    let position = |i: u32| {
        usize::try_from(i)
            .ok()
            .and_then(|i| vertices.get(i))
            .copied()
    };
    let (mut walls, mut wall_faces) = (Vec::new(), Vec::new());
    let (mut top, mut bottom): (Vec<[u32; 3]>, Vec<[u32; 3]>) = (Vec::new(), Vec::new());
    let (mut top_faces, mut bottom_faces) = (Vec::new(), Vec::new());
    for (tri, face) in mesh.triangles().iter().zip(mesh.faces()) {
        let [a, b, c] = tri.map(|i| position(i).ok_or_else(|| not_prism(OpDefect::BodyNotPrism)));
        let (a, b, c) = (a?, b?, c?);
        let orientation = area_sign(&[[a[0], a[1]], [b[0], b[1]], [c[0], c[1]]], budget)?;
        if a[2] == z1 && b[2] == z1 && c[2] == z1 {
            if orientation <= 0 {
                return Err(not_prism(OpDefect::BodyNotPrism));
            }
            top.push(*tri);
            top_faces.push(*face);
        } else if a[2] == z0 && b[2] == z0 && c[2] == z0 {
            if orientation >= 0 {
                return Err(not_prism(OpDefect::BodyNotPrism));
            }
            bottom.push(*tri);
            bottom_faces.push(*face);
        } else {
            if orientation != 0 {
                return Err(not_prism(OpDefect::BodyNotPrism));
            }
            walls.push(*tri);
            wall_faces.push(*face);
        }
    }
    let single = |faces: &[u16]| match faces.split_first() {
        Some((first, rest)) if rest.iter().all(|f| f == first) => Some(*first),
        _ => None,
    };
    let (Some(top_face), Some(bottom_face)) = (single(&top_faces), single(&bottom_faces)) else {
        return Err(not_prism(OpDefect::BodyCapFaces));
    };
    if top_face == bottom_face {
        return Err(not_prism(OpDefect::BodyCapFaces));
    }
    let top_outline = outline(&top, false, &position, limits, budget)?;
    let bottom_outline = outline(&bottom, true, &position, limits, budget)?;
    // With every other triangle vertical and the body certified, both caps cover the same
    // region; only the loop structure is compared here (the two caps may carry different flat
    // vertices on the same straight outline, and the final certification proves the rest).
    if top_outline.1.len() != bottom_outline.1.len() {
        return Err(not_prism(OpDefect::BodyNotPrism));
    }
    Ok(Prism {
        z0,
        z1,
        top_face,
        bottom_face,
        top: top_outline,
        bottom: bottom_outline,
        walls,
        wall_faces,
    })
}

// The outer loop (counter-clockwise from +Z) and the holes (clockwise) of one cap, from the
// boundary of its triangles; a bottom cap's edges are reversed to the same convention.
fn outline(
    triangles: &[[u32; 3]],
    reverse: bool,
    position: &impl Fn(u32) -> Option<[f64; 3]>,
    limits: &OpLimits,
    budget: &mut ExactBudget,
) -> Result<(CapLoop, Vec<CapLoop>), OpFailure> {
    let not_prism = |defect| OpFailure::unsupported(OpField::Body, defect, Stage::Build);
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
        _ => not_prism(OpDefect::BodyLoops),
    })?;
    let loops: Vec<CapLoop> = rings
        .iter()
        .map(|r| canonical_loop(r, position))
        .collect::<Option<_>>()
        .ok_or_else(|| not_prism(OpDefect::BodyLoops))?;
    split_loops(
        loops,
        |p| [p[0], p[1]],
        OpDefect::BodyLoops,
        OpField::Body,
        budget,
    )
}
