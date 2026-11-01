// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! G13, slice 1: a ruled surface between two planar contours, with thickness.
//!
//! Two simple counter-clockwise contours with the same vertex count form one closed
//! solid: every vertex pair `start[i]`, `end[i]` is joined by one straight ruling, the
//! wall quads follow the contour edges, and the two caps are the G05 contour
//! triangulations. The sheet thickness is the distance between the contours themselves
//! (a bent plate states its outer and inner bend contours, a lofted plate its two face
//! contours), so no separate thickness input exists — see the design note
//! `docs/design/g13-ruled-surfaces.md`.
//!
//! Cylinder and cone patches are the two degenerate lofts: a circular-arc contour
//! translated along the cylinder axis, and two arcs of different radii about the same
//! cone axis. Arc contours are polygons with vertices **on** the true curve, flagged by a
//! proven sagitta bound (the G05 linear-interpolation bound) that the result reports.
//! [`cylinder_patch`] and [`cone_patch`] build both patches first-class from authored
//! bend radii and thickness: the kernel derives the two matched annular-sector sections
//! on one angular grid, so the caller never resamples arcs by hand.
//!
//! Fold lines are the transverse wall edges at which the sheet changes direction: the
//! vertex indices where either contour does not continue straight. A bent plate reads
//! them as its bend lines; a faceted arc reports none (its turns belong to the flagged
//! approximation, not to authored folds).
//!
//! The candidate is sealed by the G09 machinery, which certifies the closed outward
//! 2-manifold shell, refuses any self-intersecting (twisted) correspondence, and carries
//! the analytic volume self-check: for an exact translation of one contour (every ruling
//! equal) the body is an oblique prism with the exact volume `area * |b . n|`. Other
//! analytical references (Pappus-type cylinder, cone, and general parallels) are
//! independent closed forms in the fixtures, as for the G12 sweep.

use crate::kernel::sweep::quad_area_vector;
use beamcraft_geom::tolerance::{COORDINATE_CAP_MM, DIMENSION_CAP_MM};
use beamcraft_geom::{Frame3, Point3, Vec3};
use beamcraft_kernel::enclosure::Enclosure;
use beamcraft_kernel::ops::seal::{Candidate, Expect, OpKind, seal};
use beamcraft_kernel::ops::{OpFailure, OpLimits, OpResult};
use beamcraft_kernel::profile::diagnostic::ProfileFailure;
use beamcraft_kernel::profile::{CurveSpec, Point2, Profile, choose_segments};
use beamcraft_model::error::ErrorCode;
use core::f64::consts::TAU;

/// Vertex budget of one loft mesh: the F06 mesh cap of 4096 vertices, checked with
/// integer arithmetic before any allocation (two rings, so at most 2048 points per
/// contour).
pub const MAX_LOFT_VERTICES: u64 = 4096;

/// Triangle budget of one loft mesh: the F06 mesh cap of 8192 triangles.
pub const MAX_LOFT_TRIANGLES: u64 = 8192;

/// A vertex whose turn satisfies `|cross|/(|a||b|) <= 1e-9` continues straight (same
/// bound as a G12 mitre) and is not a fold line.
pub const MIN_FOLD_CROSS: f64 = 1e-9;

/// Why a contour input was refused; every token reuses established F06 vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoftCode {
    /// NaN or infinity in an origin, axis, radius, angle or sagitta.
    NonFinite,
    /// A coordinate, radius or sagitta above its cap.
    AboveLimit,
    /// A contour with fewer than three points.
    TooFewPoints,
    /// The two contours have different vertex counts; no index correspondence exists.
    ContourMismatch,
    /// A requested arc sagitta is zero or negative.
    BadDeviation,
    /// The requested sagitta is unreachable inside the G05 segment cap.
    DeviationUnreachable,
    /// The mesh would exceed the loft vertex or triangle budget.
    TooManyTriangles,
    /// A derived frame, position or fold measure could not be certified.
    NumericUncertain,
}

impl LoftCode {
    /// Stable wire token; only established F06 and proposed `GEO_OP_` tokens are reused.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NonFinite => "GEO_NON_FINITE",
            Self::AboveLimit | Self::DeviationUnreachable => "GEO_ABOVE_LIMIT",
            Self::TooFewPoints | Self::ContourMismatch => "GEO_CONTOUR_INVALID",
            Self::BadDeviation => "GEO_NOT_POSITIVE",
            Self::TooManyTriangles => "GEO_BUFFER_LIMIT",
            Self::NumericUncertain => "GEO_NUMERIC_UNCERTAIN",
        }
    }

    /// Existing F04 code of the refusal.
    pub const fn f04_code(self) -> ErrorCode {
        match self {
            Self::NonFinite => ErrorCode::NonFiniteNumber,
            Self::AboveLimit | Self::DeviationUnreachable | Self::TooManyTriangles => {
                ErrorCode::LimitExceeded
            }
            Self::TooFewPoints | Self::ContourMismatch | Self::BadDeviation => {
                ErrorCode::InvalidArgument
            }
            Self::NumericUncertain => ErrorCode::UnsupportedGeometry,
        }
    }
}

/// Why a loft could not be built: the failure of the stage that refused it.
#[derive(Debug, Clone, PartialEq)]
pub enum LoftFailure {
    /// The contours were refused before any admission work.
    Contour(LoftCode),
    /// G05 refused a contour's admission or its cap triangulation.
    Profile(ProfileFailure),
    /// The G09 seal refused the candidate mesh or its volume self-check.
    Body(OpFailure),
}

impl LoftFailure {
    /// The F04 code of the failure.
    pub fn f04_code(&self) -> ErrorCode {
        match self {
            Self::Contour(code) => code.f04_code(),
            Self::Profile(failure) => failure
                .diagnostics()
                .first()
                .map_or(ErrorCode::UnsupportedGeometry, |d| d.code.f04_code()),
            Self::Body(failure) => failure.code.f04_code(),
        }
    }
}

fn numeric() -> LoftFailure {
    LoftFailure::Contour(LoftCode::NumericUncertain)
}

/// One planar contour: counter-clockwise `points` in the certified orthonormal `frame`
/// about `origin` (the frame X and Y axes span the contour plane, Z is its outward
/// normal). Planarity is exact by construction: every vertex is
/// `origin + x * X + y * Y`.
#[derive(Debug, Clone)]
pub struct RuledContour {
    origin: Point3,
    frame: Frame3,
    points: Vec<Point2>,
    /// Proven upper bound of the distance between the polygon and the true arc, when the
    /// contour is a generated arc (millimetres).
    sagitta_bound_mm: Option<f64>,
}

impl RuledContour {
    /// An authored polygonal contour. The axes need not be unit or orthogonal —
    /// `Frame3::try_new` certifies the orthonormal frame built from their directions
    /// (X, then the component of Y across X) — and admission validates the points when
    /// the loft is built.
    pub fn polygon(
        origin: Point3,
        x_axis: Vec3,
        y_axis: Vec3,
        points: Vec<Point2>,
    ) -> Result<Self, LoftFailure> {
        if points.len() < 3 {
            return Err(LoftFailure::Contour(LoftCode::TooFewPoints));
        }
        let frame = frame_of(origin, x_axis, y_axis)?;
        Ok(Self {
            origin,
            frame,
            points,
            sagitta_bound_mm: None,
        })
    }

    /// A circular-arc contour: `radius_mm` about the frame origin `centre` in the frame
    /// plane, from `start_rad` travelling counter-clockwise over `sweep_rad` (a full
    /// turn of exactly `TAU` produces the closed circle without a duplicated point).
    /// The vertices lie on the true arc; the reported sagitta bound is the G05
    /// linear-interpolation bound `r * phi^2 / 8` of the chosen facet angle.
    pub fn arc(
        centre: Point3,
        x_axis: Vec3,
        y_axis: Vec3,
        radius_mm: f64,
        start_rad: f64,
        sweep_rad: f64,
        max_sagitta_mm: f64,
    ) -> Result<Self, LoftFailure> {
        if !radius_mm.is_finite() || !start_rad.is_finite() {
            return Err(LoftFailure::Contour(LoftCode::NonFinite));
        }
        if radius_mm <= 0.0 {
            return Err(LoftFailure::Contour(LoftCode::BadDeviation));
        }
        if radius_mm > DIMENSION_CAP_MM {
            return Err(LoftFailure::Contour(LoftCode::AboveLimit));
        }
        let deviation = checked_deviation(max_sagitta_mm)?;
        if !(sweep_rad.is_finite() && sweep_rad > 0.0 && sweep_rad <= TAU) {
            return Err(LoftFailure::Contour(LoftCode::BadDeviation));
        }
        let frame = frame_of(centre, x_axis, y_axis)?;
        let spec = CurveSpec {
            max_sagitta_mm: deviation,
            min_segments: 8,
            max_segments: 1020,
        };
        let (per_turn, _) = choose_segments(radius_mm, spec)
            .map_err(|_| LoftFailure::Contour(LoftCode::DeviationUnreachable))?;
        let step = TAU / f64::from(per_turn);
        let facets = (sweep_rad / step).ceil();
        if !facets.is_finite() || facets > 1021.0 {
            return Err(LoftFailure::Contour(LoftCode::DeviationUnreachable));
        }
        let facet_angle = sweep_rad / facets;
        let full_turn = sweep_rad == TAU;
        let count = if full_turn {
            facets as u64
        } else {
            facets as u64 + 1
        };
        let mut points = Vec::new();
        for j in 0..count {
            let phi = start_rad + facet_angle * j as f64;
            let (sin, cos) = phi.sin_cos();
            points.push(Point2::new(radius_mm * cos, radius_mm * sin));
        }
        Ok(Self {
            origin: centre,
            frame,
            points,
            sagitta_bound_mm: Some(radius_mm * facet_angle * facet_angle / 8.0),
        })
    }

    /// The contour origin.
    pub const fn origin(&self) -> Point3 {
        self.origin
    }

    /// The certified contour frame; Z is the outward plane normal.
    pub const fn frame(&self) -> Frame3 {
        self.frame
    }

    /// The contour points, counter-clockwise in the frame plane.
    pub fn points(&self) -> &[Point2] {
        &self.points
    }

    /// Proven upper bound of the arc-faceting distance (millimetres); `None` for an
    /// authored polygon.
    pub const fn sagitta_bound_mm(&self) -> Option<f64> {
        self.sagitta_bound_mm
    }
}

/// Certifies the orthonormal contour frame from two in-plane axis directions.
fn frame_of(origin: Point3, x_axis: Vec3, y_axis: Vec3) -> Result<Frame3, LoftFailure> {
    origin
        .validated()
        .map_err(|_| LoftFailure::Contour(LoftCode::NonFinite))?;
    if origin.x.abs() > COORDINATE_CAP_MM
        || origin.y.abs() > COORDINATE_CAP_MM
        || origin.z.abs() > COORDINATE_CAP_MM
    {
        return Err(LoftFailure::Contour(LoftCode::AboveLimit));
    }
    let x = x_axis
        .normalized()
        .map_err(|_| LoftFailure::Contour(LoftCode::NumericUncertain))?;
    let z = x
        .cross(y_axis)
        .map_err(|_| LoftFailure::Contour(LoftCode::NumericUncertain))?
        .normalized()
        .map_err(|_| LoftFailure::Contour(LoftCode::NumericUncertain))?;
    let y = z
        .cross(x)
        .map_err(|_| LoftFailure::Contour(LoftCode::NumericUncertain))?;
    Frame3::try_new(x, y, z).map_err(|_| LoftFailure::Contour(LoftCode::NumericUncertain))
}

fn checked_deviation(deviation: f64) -> Result<f64, LoftFailure> {
    if !deviation.is_finite() {
        return Err(LoftFailure::Contour(LoftCode::NonFinite));
    }
    if deviation <= 0.0 {
        return Err(LoftFailure::Contour(LoftCode::BadDeviation));
    }
    if deviation > DIMENSION_CAP_MM {
        return Err(LoftFailure::Contour(LoftCode::AboveLimit));
    }
    Ok(deviation)
}

/// Input of a ruled loft: two contours with the same vertex count.
#[derive(Debug, Clone)]
pub struct Loft {
    /// The start contour; its cap closes the body against the start normals.
    pub start: RuledContour,
    /// The end contour, joined to `start` by the index correspondence of the rulings.
    pub end: RuledContour,
}

/// A sealed loft: a certified body with its blob, digest, measurements and fold lines.
#[derive(Debug, Clone)]
pub struct LoftResult {
    op: OpResult,
    area_mm2: Option<Enclosure>,
    sagitta_bound_mm: Option<f64>,
    fold_vertices: Vec<u32>,
}

impl LoftResult {
    /// The certified body (mesh, face table, normals, certificate and volume enclosure).
    pub const fn body(&self) -> &beamcraft_kernel::ops::Body {
        self.op.body()
    }

    /// The sealed operation result the loft wrapped, for readers that consume an `OpResult`.
    pub const fn op(&self) -> &beamcraft_kernel::ops::OpResult {
        &self.op
    }

    /// The sealed BCMESH01 blob of the loft mesh.
    pub fn blob(&self) -> &[u8] {
        self.op.blob()
    }

    /// SHA-256 of the blob.
    pub const fn mesh_digest(&self) -> beamcraft_model::digest::ContentDigest {
        self.op.mesh_digest()
    }

    /// Enclosure of the mesh volume in mm3 (exact signed-volume sum, enclosed once).
    pub const fn volume_mm3(&self) -> Enclosure {
        self.op.volume_mm3()
    }

    /// Enclosure of the mesh volume in m3.
    pub fn volume_m3(&self) -> Option<Enclosure> {
        self.op.volume_m3()
    }

    /// Enclosure of the mesh surface area in mm2 (G06 exact moment sum, enclosed once);
    /// `None` when the measurement was not certified.
    pub const fn area_mm2(&self) -> Option<Enclosure> {
        self.area_mm2
    }

    /// Exact vertex bounds of the mesh.
    pub const fn bounds(&self) -> &beamcraft_kernel::spatial::Aabb {
        self.op.bounds()
    }

    /// The larger proven sagitta bound of the two contours (millimetres); `None` when
    /// both are authored polygons.
    pub const fn sagitta_bound_mm(&self) -> Option<f64> {
        self.sagitta_bound_mm
    }

    /// Sorted vertex indices whose transverse wall edge is a fold line: the ruling at
    /// that index crosses a contour vertex where either contour does not continue
    /// straight (see [`MIN_FOLD_CROSS`]).
    pub fn fold_vertices(&self) -> &[u32] {
        &self.fold_vertices
    }

    /// Work counters of the seal.
    pub const fn stats(&self) -> &beamcraft_kernel::ops::OpStats {
        self.op.stats()
    }
}

/// Places one contour into space: every point through the certified frame, coordinates
/// checked against the cap.
fn placed(contour: &RuledContour) -> Result<Vec<[f64; 3]>, LoftFailure> {
    let mut out = Vec::new();
    for point in contour.points() {
        let local = Vec3::new(point.x, point.y, 0.0)
            .map_err(|_| LoftFailure::Contour(LoftCode::NonFinite))?;
        let moved = contour.frame().map_vector(local).map_err(|_| numeric())?;
        let world = contour.origin().translated(moved).map_err(|_| numeric())?;
        if world.x.abs() > COORDINATE_CAP_MM
            || world.y.abs() > COORDINATE_CAP_MM
            || world.z.abs() > COORDINATE_CAP_MM
        {
            return Err(LoftFailure::Contour(LoftCode::AboveLimit));
        }
        out.push([world.x, world.y, world.z]);
    }
    Ok(out)
}

/// The fold vertices of one contour: indices whose turn exceeds [`MIN_FOLD_CROSS`].
fn folds(points: &[Point2]) -> Result<Vec<u32>, LoftFailure> {
    let n = points.len();
    let mut out = Vec::new();
    for i in 0..n {
        let prev = points.get((i + n - 1) % n).copied().ok_or_else(numeric)?;
        let cur = points.get(i).copied().ok_or_else(numeric)?;
        let next = points.get((i + 1) % n).copied().ok_or_else(numeric)?;
        let (ax, ay) = (cur.x - prev.x, cur.y - prev.y);
        let (bx, by) = (next.x - cur.x, next.y - cur.y);
        let cross = ax * by - ay * bx;
        let denom = (ax * ax + ay * ay).sqrt() * (bx * bx + by * by).sqrt();
        if !cross.is_finite() || !denom.is_finite() {
            return Err(numeric());
        }
        if denom > 0.0 && cross.abs() / denom > MIN_FOLD_CROSS {
            out.push(u32::try_from(i).map_err(|_| numeric())?);
        }
    }
    Ok(out)
}

/// The analytic volume of an exact translation: when every ruling is bit-identical to
/// the same vector, the body is an oblique prism of the start contour's section over the
/// height `|b . n|` (Cavalieri), exact in real arithmetic. `None` for any other pair.
fn translation_volume(
    ring0: &[[f64; 3]],
    ring1: &[[f64; 3]],
    normal: Vec3,
    area: f64,
) -> Option<f64> {
    let first = ring0.first()?;
    let last = ring1.first()?;
    let (bx, by, bz) = (last[0] - first[0], last[1] - first[1], last[2] - first[2]);
    for (a, b) in ring0.iter().zip(ring1.iter()) {
        if b[0] - a[0] != bx || b[1] - a[1] != by || b[2] - a[2] != bz {
            return None;
        }
    }
    let height = (normal.x() * bx + normal.y() * by + normal.z() * bz).abs();
    let volume = area * height;
    if height > 0.0 && volume.is_finite() {
        Some(volume)
    } else {
        None
    }
}

/// One cap triangle's vertex indices, widened for vertex access.
fn tri_points(cap: &[u32; 3]) -> Result<(usize, usize, usize), LoftFailure> {
    let (a, b, c) = (
        cap.first().copied().ok_or_else(numeric)?,
        cap.get(1).copied().ok_or_else(numeric)?,
        cap.get(2).copied().ok_or_else(numeric)?,
    );
    Ok((
        usize::try_from(a).map_err(|_| numeric())?,
        usize::try_from(b).map_err(|_| numeric())?,
        usize::try_from(c).map_err(|_| numeric())?,
    ))
}

/// `usize` mesh index narrowed to the BCMESH01 `u32` indices.
fn to_u32(v: usize) -> Result<u32, LoftFailure> {
    u32::try_from(v).map_err(|_| numeric())
}

/// Splits one wall quad along the diagonal that stays inside it. A skew correspondence
/// can make the quad non-convex (its projection folds), where exactly one diagonal is
/// interior: the sign of the cross products against the quad's own area vector decides.
/// A degenerate quad keeps the default split, which the certification refuses anyway.
fn wall_triangles(
    vertices: &[[f64; 3]],
    a: usize,
    a2: usize,
    b2: usize,
    b: usize,
) -> Result<([usize; 3], [usize; 3]), LoftFailure> {
    let get = |i: usize| vertices.get(i).copied().ok_or_else(numeric);
    let (p0, p1, p2, p3) = (get(a)?, get(a2)?, get(b2)?, get(b)?);
    let sub = |p: [f64; 3], q: [f64; 3]| [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
    let cross = |u: [f64; 3], w: [f64; 3]| {
        [
            u[1] * w[2] - u[2] * w[1],
            u[2] * w[0] - u[0] * w[2],
            u[0] * w[1] - u[1] * w[0],
        ]
    };
    let dot = |u: [f64; 3], w: [f64; 3]| u[0] * w[0] + u[1] * w[1] + u[2] * w[2];
    // A crossed (butterfly) quad has a zero area vector; any split of it fails the
    // certification, so the default split is as good as any.
    let n = quad_area_vector(vertices, a, a2, b2, b).unwrap_or([0.0; 3]);
    let d0 = sub(p2, p0);
    let s1 = dot(cross(d0, sub(p1, p0)), n);
    let s2 = dot(cross(d0, sub(p3, p0)), n);
    if !s1.is_finite() || !s2.is_finite() {
        return Err(numeric());
    }
    if s1 * s2 < 0.0 {
        Ok(([a, a2, b2], [a, b2, b]))
    } else {
        Ok(([a2, b2, b], [a2, b, a]))
    }
}

/// Builds the candidate: one wall quad per contour edge (a face with its area-vector
/// normal), then the two caps from the G05 triangulations (the start cap reversed).
fn build_mesh(
    ring0: &[[f64; 3]],
    ring1: &[[f64; 3]],
    caps0: &[[u32; 3]],
    caps1: &[[u32; 3]],
    z0: Vec3,
    z1: Vec3,
) -> Result<Candidate, LoftFailure> {
    let count = ring0.len();
    let mut vertices = Vec::new();
    vertices.extend(ring0.iter().copied());
    vertices.extend(ring1.iter().copied());
    let mut triangles = Vec::new();
    let mut faces = Vec::new();
    let mut ids = Vec::new();
    let mut normals = Vec::new();
    for i in 0..count {
        let i2 = (i + 1) % count;
        let (a, a2, b, b2) = (i, i2, count + i, count + i2);
        let (first, second) = wall_triangles(&vertices, a, a2, b2, b)?;
        triangles.push([to_u32(first[0])?, to_u32(first[1])?, to_u32(first[2])?]);
        triangles.push([to_u32(second[0])?, to_u32(second[1])?, to_u32(second[2])?]);
        let face = u16::try_from(ids.len()).map_err(|_| numeric())?;
        faces.push(face);
        faces.push(face);
        ids.push(format!("ruled:w{i}"));
        normals.push(quad_area_vector(&vertices, a, a2, b2, b));
    }
    let cap_normal = |z: Vec3, sign: f64| -> Option<[f64; 3]> {
        Some([sign * z.x(), sign * z.y(), sign * z.z()])
    };
    let start_z = cap_normal(z0, -1.0);
    let end_z = cap_normal(z1, 1.0);
    // One face id per logical cap: the triangles of a cap share the id, as the face table
    // requires one entry per logical face with no duplicated id.
    let face = u16::try_from(ids.len()).map_err(|_| numeric())?;
    ids.push("ruled:f.cap0".to_owned());
    normals.push(start_z);
    for cap in caps0 {
        let (a, b, c) = tri_points(cap)?;
        // The start cap is the same triangulation wound the other way: its outward
        // normal points against the start frame Z.
        triangles.push([to_u32(a)?, to_u32(c)?, to_u32(b)?]);
        faces.push(face);
    }
    let face = u16::try_from(ids.len()).map_err(|_| numeric())?;
    ids.push("ruled:f.cap1".to_owned());
    normals.push(end_z);
    for cap in caps1 {
        let (a, b, c) = tri_points(cap)?;
        triangles.push([to_u32(a + count)?, to_u32(b + count)?, to_u32(c + count)?]);
        faces.push(face);
    }
    Ok(Candidate {
        vertices,
        triangles,
        faces,
        face_ids: ids,
        normals,
    })
}

/// Lofts the two contours into one sealed certified solid.
///
/// Stages: the G05 admission of both contours, the vertex-count correspondence, the cap
/// triangulations, the mesh budget, the placed rings and fold lines, and the G09 seal —
/// which certifies the closed shell and cross-checks the mesh volume against the
/// analytic prism volume for an exact translation.
pub fn loft(input: &Loft) -> Result<LoftResult, LoftFailure> {
    let profile0 = Profile::admit(input.start.points(), &[]).map_err(LoftFailure::Profile)?;
    let profile1 = Profile::admit(input.end.points(), &[]).map_err(LoftFailure::Profile)?;
    if profile0.outer().len() != profile1.outer().len() {
        return Err(LoftFailure::Contour(LoftCode::ContourMismatch));
    }
    let caps0 = profile0.triangulate().map_err(LoftFailure::Profile)?;
    let caps1 = profile1.triangulate().map_err(LoftFailure::Profile)?;
    let count = u64::try_from(profile0.outer().len()).map_err(|_| numeric())?;
    let caps0_n = u64::try_from(caps0.len()).map_err(|_| numeric())?;
    let caps1_n = u64::try_from(caps1.len()).map_err(|_| numeric())?;
    let vertices = 2u64
        .checked_mul(count)
        .ok_or(LoftFailure::Contour(LoftCode::TooManyTriangles))?;
    let triangles = 2u64
        .checked_mul(count)
        .and_then(|walls| walls.checked_add(caps0_n))
        .and_then(|sum| sum.checked_add(caps1_n))
        .ok_or(LoftFailure::Contour(LoftCode::TooManyTriangles))?;
    if vertices > MAX_LOFT_VERTICES || triangles > MAX_LOFT_TRIANGLES {
        return Err(LoftFailure::Contour(LoftCode::TooManyTriangles));
    }
    let ring0 = placed(&input.start)?;
    let ring1 = placed(&input.end)?;
    // Fold lines come from authored contours only: the turns of a faceted arc belong to
    // the flagged approximation, not to an authored fold of the sheet.
    let mut fold_vertices = Vec::new();
    if input.start.sagitta_bound_mm().is_none() {
        fold_vertices.extend(folds(input.start.points())?);
    }
    if input.end.sagitta_bound_mm().is_none() {
        fold_vertices.extend(folds(input.end.points())?);
    }
    fold_vertices.sort_unstable();
    fold_vertices.dedup();
    let z0 = input.start.frame().z();
    let z1 = input.end.frame().z();
    // The larger geometric floor of the two contours governs the seal's tolerance.
    let tolerance = if profile0.tolerance().eps_geom_mm() >= profile1.tolerance().eps_geom_mm() {
        profile0.tolerance()
    } else {
        profile1.tolerance()
    };
    let analytic = match translation_volume(&ring0, &ring1, z0, profile0.net_area_mm2()) {
        Some(predicted) => {
            let slack = 1e-9 * predicted.abs() + 1e-6;
            Some(Enclosure::new(predicted - slack, predicted + slack).ok_or_else(numeric)?)
        }
        None => None,
    };
    let candidate = build_mesh(&ring0, &ring1, &caps0, &caps1, z0, z1)?;
    let expect = Expect {
        kind: OpKind::Ruled,
        euler: None,
        genus: Some(0),
        analytic_volume: analytic,
        plane_deviation_mm: None,
        stats: Default::default(),
    };
    let op = seal(
        candidate,
        tolerance,
        &OpLimits::DEFAULT,
        expect,
        &mut |_| false,
    )
    .map_err(LoftFailure::Body)?;
    let area_mm2 = beamcraft_kernel::measure::measure(op.body().mesh(), op.body().analysis())
        .ok()
        .and_then(|m| m.area_mm2());
    let sagitta_bound_mm = match (input.start.sagitta_bound_mm(), input.end.sagitta_bound_mm()) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) | (None, Some(a)) => Some(a),
        (None, None) => None,
    };
    Ok(LoftResult {
        op,
        area_mm2,
        sagitta_bound_mm,
        fold_vertices,
    })
}

/// An authored cylindrical patch: the bent sheet of thickness `thickness_mm` between
/// the inner bend radius `radius_mm` and the outer radius `radius_mm + thickness_mm`,
/// swept `sweep_rad` about the Z axis of the certified frame from `start_rad`, running
/// `height_mm` along that axis from the frame plane through `origin`.
///
/// The section (an annular sector) is one counter-clockwise polygon on a single angular
/// grid chosen from the outer radius, so the start and end contours share the vertex
/// count and the correspondence is the continuous one. The sweep is a partial turn,
/// strictly below a full turn: a closed ring needs a cavity section, which the loft
/// does not admit (as the A10 roll). The vertices lie on the true arcs; the proven
/// sagitta bound is reported and the faceted turns contribute no fold lines.
#[derive(Debug, Clone)]
pub struct CylinderPatch {
    /// A point on the axis in the sheet's lower plane; the frame X and Y span the
    /// section plane, Z is the axis direction.
    pub origin: Point3,
    /// A section-plane direction, certified into the frame X axis.
    pub x_axis: Vec3,
    /// A second section-plane direction, certified into the frame Y axis.
    pub y_axis: Vec3,
    /// The authored inner bend radius (millimetres).
    pub radius_mm: f64,
    /// The sheet thickness (millimetres); the outer radius is `radius_mm + thickness_mm`.
    pub thickness_mm: f64,
    /// The start angle of the sweep, radians about the axis.
    pub start_rad: f64,
    /// The swept angle, radians, strictly inside `(0, TAU)`.
    pub sweep_rad: f64,
    /// The patch height along the frame Z axis (millimetres).
    pub height_mm: f64,
    /// The largest accepted chord sagitta of the faceted arcs (millimetres).
    pub max_sagitta_mm: f64,
}

/// An authored conical patch: the bent sheet of thickness `thickness_mm` whose inner
/// face tapers from `radius_mm` at the frame plane to `end_radius_mm` `height_mm` along
/// the frame axis. Everything else as for [`CylinderPatch`]; with
/// `end_radius_mm == radius_mm` the bytes are the cylinder patch's.
#[derive(Debug, Clone)]
pub struct ConePatch {
    /// A point on the axis in the sheet's lower plane; the frame X and Y span the
    /// section plane, Z is the axis direction.
    pub origin: Point3,
    /// A section-plane direction, certified into the frame X axis.
    pub x_axis: Vec3,
    /// A second section-plane direction, certified into the frame Y axis.
    pub y_axis: Vec3,
    /// The inner bend radius at the frame plane (millimetres).
    pub radius_mm: f64,
    /// The inner bend radius at the far plane (millimetres).
    pub end_radius_mm: f64,
    /// The sheet thickness (millimetres), measured radially at both levels.
    pub thickness_mm: f64,
    /// The start angle of the sweep, radians about the axis.
    pub start_rad: f64,
    /// The swept angle, radians, strictly inside `(0, TAU)`.
    pub sweep_rad: f64,
    /// The patch height along the frame Z axis (millimetres).
    pub height_mm: f64,
    /// The largest accepted chord sagitta of the faceted arcs (millimetres).
    pub max_sagitta_mm: f64,
}

/// Validates one authored radius, thickness or height: finite, positive, below the
/// dimension cap.
fn checked_positive(value: f64) -> Result<f64, LoftFailure> {
    if !value.is_finite() {
        return Err(LoftFailure::Contour(LoftCode::NonFinite));
    }
    if value <= 0.0 {
        return Err(LoftFailure::Contour(LoftCode::BadDeviation));
    }
    if value > DIMENSION_CAP_MM {
        return Err(LoftFailure::Contour(LoftCode::AboveLimit));
    }
    Ok(value)
}

/// Validates a patch sweep: a finite start angle and a sweep strictly inside a full
/// turn (the closed ring of a full turn needs a cavity section the loft does not take).
fn checked_sweep(start_rad: f64, sweep_rad: f64) -> Result<(), LoftFailure> {
    if !start_rad.is_finite() || !sweep_rad.is_finite() {
        return Err(LoftFailure::Contour(LoftCode::NonFinite));
    }
    if !(sweep_rad > 0.0 && sweep_rad < TAU) {
        return Err(LoftFailure::Contour(LoftCode::BadDeviation));
    }
    Ok(())
}

/// The angular grid of a patch: the facet count over the sweep follows the G05 segment
/// choice for `r_max`, the largest radius of the four skins (so the reported bound meets
/// the request), exactly as `RuledContour::arc` chooses for one arc. Returns the facet
/// count, the facet angle and the proven sagitta bound `r_max * phi^2 / 8`.
fn patch_grid(
    r_max: f64,
    sweep_rad: f64,
    max_sagitta_mm: f64,
) -> Result<(u64, f64, f64), LoftFailure> {
    let spec = CurveSpec {
        max_sagitta_mm,
        min_segments: 8,
        max_segments: 1020,
    };
    let (per_turn, _) = choose_segments(r_max, spec)
        .map_err(|_| LoftFailure::Contour(LoftCode::DeviationUnreachable))?;
    let step = TAU / f64::from(per_turn);
    let facets = (sweep_rad / step).ceil();
    if !facets.is_finite() || facets > 1021.0 {
        return Err(LoftFailure::Contour(LoftCode::DeviationUnreachable));
    }
    let facet_angle = sweep_rad / facets;
    Ok((
        facets as u64,
        facet_angle,
        r_max * facet_angle * facet_angle / 8.0,
    ))
}

/// The `facets + 1` points on the true circle of `radius_mm`, from `start_rad` in
/// `facet_angle` steps; `reverse` walks the angles back down.
fn arc_points(
    radius_mm: f64,
    start_rad: f64,
    facets: u64,
    facet_angle: f64,
    reverse: bool,
) -> Vec<Point2> {
    let mut out = Vec::new();
    for j in 0..=facets {
        let k = if reverse { facets - j } else { j };
        let phi = start_rad + facet_angle * k as f64;
        let (sin, cos) = phi.sin_cos();
        out.push(Point2::new(radius_mm * cos, radius_mm * sin));
    }
    out
}

/// The annular-sector section polygon of one patch level: the outer arc
/// counter-clockwise from the start angle, the implicit radial edge in, the inner arc
/// back, and the closing radial edge out — material on the left of every edge.
fn sector_section(
    r_in: f64,
    r_out: f64,
    start_rad: f64,
    facets: u64,
    facet_angle: f64,
) -> Vec<Point2> {
    let outer = arc_points(r_out, start_rad, facets, facet_angle, false);
    let inner = arc_points(r_in, start_rad, facets, facet_angle, true);
    let mut points = outer;
    points.extend(inner);
    points
}

/// A generated arc-grid polygon: like [`RuledContour::polygon`], but carrying the
/// proven `sagitta_bound_mm` of its faceting — the loft reports the deviation and reads
/// no authored fold lines from the flagged turns.
fn flagged_contour(
    origin: Point3,
    x_axis: Vec3,
    y_axis: Vec3,
    points: Vec<Point2>,
    sagitta_bound_mm: f64,
) -> Result<RuledContour, LoftFailure> {
    let mut contour = RuledContour::polygon(origin, x_axis, y_axis, points)?;
    contour.sagitta_bound_mm = Some(sagitta_bound_mm);
    Ok(contour)
}

/// Builds one patch: the two level sections `(inner, outer)` radii on one angular grid,
/// the upper translated `height_mm` along the frame axis, sealed through [`loft`]. For
/// equal level sections the translation is exact, so the seal carries the analytic
/// prism self-check (`section area x height`).
#[allow(clippy::too_many_arguments)]
fn patch_loft(
    origin: Point3,
    x_axis: Vec3,
    y_axis: Vec3,
    start_radii: (f64, f64),
    end_radii: (f64, f64),
    start_rad: f64,
    sweep_rad: f64,
    height_mm: f64,
    max_sagitta_mm: f64,
) -> Result<LoftResult, LoftFailure> {
    let frame = frame_of(origin, x_axis, y_axis)?;
    let deviation = checked_deviation(max_sagitta_mm)?;
    // The derived outer radii keep the envelope `RuledContour::arc` enforces on one
    // arc: no skin may sit beyond the dimension cap even when radius and thickness
    // each pass their own check.
    let r_max = start_radii.1.max(end_radii.1);
    if r_max > DIMENSION_CAP_MM {
        return Err(LoftFailure::Contour(LoftCode::AboveLimit));
    }
    let (facets, facet_angle, bound) = patch_grid(r_max, sweep_rad, deviation)?;
    let start = flagged_contour(
        origin,
        x_axis,
        y_axis,
        sector_section(start_radii.0, start_radii.1, start_rad, facets, facet_angle),
        bound,
    )?;
    let axis = frame.z().scaled(height_mm).map_err(|_| numeric())?;
    let end_origin = origin.translated(axis).map_err(|_| numeric())?;
    let end = flagged_contour(
        end_origin,
        x_axis,
        y_axis,
        sector_section(end_radii.0, end_radii.1, start_rad, facets, facet_angle),
        bound,
    )?;
    loft(&Loft { start, end })
}

/// Lofts an authored [`CylinderPatch`] into one sealed certified solid: the bent sheet
/// between the inner bend radius and `radius_mm + thickness_mm` about the frame axis.
/// The exact translation along the axis carries the seal's analytic prism self-check
/// (`section area x height`).
pub fn cylinder_patch(patch: &CylinderPatch) -> Result<LoftResult, LoftFailure> {
    let radius = checked_positive(patch.radius_mm)?;
    let thickness = checked_positive(patch.thickness_mm)?;
    let height = checked_positive(patch.height_mm)?;
    checked_sweep(patch.start_rad, patch.sweep_rad)?;
    patch_loft(
        patch.origin,
        patch.x_axis,
        patch.y_axis,
        (radius, radius + thickness),
        (radius, radius + thickness),
        patch.start_rad,
        patch.sweep_rad,
        height,
        patch.max_sagitta_mm,
    )
}

/// Lofts an authored [`ConePatch`] into one sealed certified solid: the bent sheet whose
/// inner face tapers from `radius_mm` to `end_radius_mm` over the height. The rulings
/// lie in the meridian planes, so the walls sit on the true cones; the fixtures compare
/// the volume with the cone-band closed form inside the faceting bounds.
pub fn cone_patch(patch: &ConePatch) -> Result<LoftResult, LoftFailure> {
    let radius = checked_positive(patch.radius_mm)?;
    let end_radius = checked_positive(patch.end_radius_mm)?;
    let thickness = checked_positive(patch.thickness_mm)?;
    let height = checked_positive(patch.height_mm)?;
    checked_sweep(patch.start_rad, patch.sweep_rad)?;
    patch_loft(
        patch.origin,
        patch.x_axis,
        patch.y_axis,
        (radius, radius + thickness),
        (end_radius, end_radius + thickness),
        patch.start_rad,
        patch.sweep_rad,
        height,
        patch.max_sagitta_mm,
    )
}
