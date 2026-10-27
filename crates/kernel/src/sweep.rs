// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! G12: a section swept along a polyline or an arc, mitred at the corners.
//!
//! The path reduces to a chord polyline (an arc is faceted first, section 3 of
//! `docs/design/g12-sweep-along-path.md`), and every interior vertex becomes a shared
//! mitre ring: the section frame rotated by half the turn angle about the chord cross
//! product, so its plane normal is the turn bisector and the two adjacent prisms close on
//! it. End rings stay perpendicular to the true end tangents. The candidate is sealed by
//! the G09 canonicalisation and certification, which also refuses any self-intersecting
//! (too sharp) mitre.
//!
//! Volumes are exact in real arithmetic: each prism segment is cut by two planes, and the
//! volume of a truncated prism is its section area times the fibre length through the
//! section centroid (the cutting positions are affine in the section coordinates). The
//! seal therefore carries that analytic value as its own volume self-check; for arcs the
//! true solid of revolution additionally has the Pappus volume `A * R_bar * theta`, which
//! the fixtures compare against the certified mesh volume within the faceting bound.

use crate::enclosure::Enclosure;
use crate::ops::seal::{Candidate, Expect, OpKind, seal};
use crate::ops::{OpFailure, OpLimits, OpResult};
use crate::profile::diagnostic::ProfileFailure;
use crate::profile::{CurveSpec, Point2, Profile, choose_segments};
use crate::section::{Representation, SectionFailure, SectionOutline, outline};
use beamcraft_geom::frame::PARALLEL_SQUARED;
use beamcraft_geom::tolerance::{COORDINATE_CAP_MM, DIMENSION_CAP_MM};
use beamcraft_geom::{Frame3, Point3, Vec3};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::section::SectionShape;
use core::f64::consts::TAU;

/// Largest number of polyline points of one path. Every valid sweep stays far below it:
/// the mesh budget below multiplies the ring count by the section point count.
pub const MAX_PATH_POINTS: usize = 4096;

/// A vertex whose unit chords satisfy `|d1 x d2| <= 1e-9` has no geometric mitre
/// (collinear continuation, or a fold-back within the same bound of 180 degrees): refused.
pub const MIN_TURN_CROSS: f64 = 1e-9;

/// Vertex budget of one sweep mesh: certification of cap-rich meshes becomes undecided
/// beyond a few thousand triangles (G07 measurements), so the sweep stops at the F06 mesh
/// caps of 4096 vertices and 8192 triangles instead of the raw blob caps.
pub const MAX_SWEEP_VERTICES: u64 = 4096;
/// Triangle budget of one sweep mesh; see [`MAX_SWEEP_VERTICES`].
pub const MAX_SWEEP_TRIANGLES: u64 = 8192;

/// Why a path was refused; every token reuses established F06 vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathCode {
    /// NaN or infinity in a path point or the chord deviation.
    NonFinite,
    /// A coordinate, count, radius or total length above its cap.
    AboveLimit,
    /// A polyline with fewer than two points.
    TooFewPoints,
    /// A chord shorter than the section's own geometric floor.
    ZeroLengthSegment,
    /// A vertex whose turn has no geometric mitre (see [`MIN_TURN_CROSS`]).
    DegenerateTurn,
    /// Three collinear or coincident arc points: no circle exists.
    CollinearArc,
    /// A chord deviation that is zero or negative.
    BadDeviation,
    /// The requested deviation is unreachable inside the G05 segment cap.
    DeviationUnreachable,
    /// The mesh would exceed the sweep vertex or triangle budget.
    TooManyVertices,
    /// A derived frame or position could not be certified.
    NumericUncertain,
}

impl PathCode {
    /// Stable wire token; only established F06 and proposed `GEO_OP_` tokens are reused.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NonFinite => "GEO_NON_FINITE",
            Self::AboveLimit | Self::DeviationUnreachable => "GEO_ABOVE_LIMIT",
            Self::TooFewPoints | Self::CollinearArc => "GEO_CONTOUR_INVALID",
            Self::ZeroLengthSegment => "GEO_AXIS_TOO_SHORT",
            Self::DegenerateTurn => "GEO_OP_UNSUPPORTED",
            Self::BadDeviation => "GEO_NOT_POSITIVE",
            Self::TooManyVertices => "GEO_BUFFER_LIMIT",
            Self::NumericUncertain => "GEO_NUMERIC_UNCERTAIN",
        }
    }

    /// Existing F04 code of the refusal.
    pub const fn f04_code(self) -> ErrorCode {
        match self {
            Self::NonFinite => ErrorCode::NonFiniteNumber,
            Self::AboveLimit | Self::DeviationUnreachable | Self::TooManyVertices => {
                ErrorCode::LimitExceeded
            }
            Self::TooFewPoints
            | Self::ZeroLengthSegment
            | Self::CollinearArc
            | Self::BadDeviation
            | Self::DegenerateTurn => ErrorCode::InvalidArgument,
            Self::NumericUncertain => ErrorCode::UnsupportedGeometry,
        }
    }
}

/// Why a sweep could not be built: the failure of the stage that refused it.
#[derive(Debug, Clone, PartialEq)]
pub enum SweepFailure {
    /// The section outline (A07.1) was refused before any path work.
    Section(SectionFailure),
    /// The path was refused.
    Path(PathCode),
    /// G05 refused the outline, its admission or its triangulation.
    Profile(ProfileFailure),
    /// The G09 seal refused the candidate mesh or its volume self-check.
    Body(OpFailure),
}

impl SweepFailure {
    /// The F04 code of the failure.
    pub fn f04_code(&self) -> ErrorCode {
        match self {
            Self::Section(failure) => match failure {
                SectionFailure::Shape(code) => *code,
                SectionFailure::Frame(f) => f
                    .diagnostics()
                    .next()
                    .map_or(ErrorCode::UnsupportedGeometry, |d| d.code.f04_code()),
                SectionFailure::Profile(p) => p
                    .diagnostics()
                    .first()
                    .map_or(ErrorCode::UnsupportedGeometry, |d| d.code.f04_code()),
                SectionFailure::Body(b) => b.code.f04_code(),
            },
            Self::Path(code) => code.f04_code(),
            Self::Profile(failure) => failure
                .diagnostics()
                .first()
                .map_or(ErrorCode::UnsupportedGeometry, |d| d.code.f04_code()),
            Self::Body(failure) => failure.code.f04_code(),
        }
    }
}

/// The path a section is swept along, in element-local millimetres.
#[derive(Debug, Clone, PartialEq)]
pub enum SweepPath {
    /// A polyline through at least two points; interior vertices are mitred.
    Polyline(Vec<Point3>),
    /// An arc from `start` to `end` passing through `on`: three distinct points fix the
    /// plane, centre, radius and travel direction.
    Arc {
        /// Arc start.
        start: Point3,
        /// A point of the arc between start and end, fixing direction and plane.
        on: Point3,
        /// Arc end.
        end: Point3,
    },
}

/// Input of a sweep: a section shape swept along a path.
#[derive(Debug, Clone, PartialEq)]
pub struct Sweep {
    /// The path.
    pub path: SweepPath,
    /// The section shape (an A07.1 family or a custom outline).
    pub shape: SectionShape,
    /// Which outline of the section to build.
    pub representation: Representation,
    /// Stated bound on the distance between a curved path and its facet chords, in
    /// millimetres; unused by a pure polyline.
    pub chord_deviation_mm: f64,
}

/// A sealed sweep: a certified body with its blob, digest and quantities.
#[derive(Debug, Clone)]
pub struct SweepResult {
    op: OpResult,
    net_area_mm2: f64,
    approximated: bool,
    segments: u32,
    deviation_bound_mm: Option<f64>,
}

impl SweepResult {
    /// The certified body (mesh, face table, normals, certificate and volume enclosure).
    pub const fn body(&self) -> &crate::ops::Body {
        self.op.body()
    }

    /// The sealed BCMESH01 blob of the sweep mesh.
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

    /// Exact vertex bounds of the mesh.
    pub const fn bounds(&self) -> &crate::spatial::Aabb {
        self.op.bounds()
    }

    /// Net area of the swept polygon, mm2 (the shoelace area of the generated outline, not
    /// the exact section).
    pub const fn net_area_mm2(&self) -> f64 {
        self.net_area_mm2
    }

    /// Whether any curve — of the section or of the path — is replaced by a polygon.
    pub const fn approximated(&self) -> bool {
        self.approximated
    }

    /// Number of prism segments: the chords of the path, arc facets included.
    pub const fn segment_count(&self) -> u32 {
        self.segments
    }

    /// Proven upper bound of the distance between the swept surface and the true arc, in
    /// millimetres; `None` for a polyline path.
    pub const fn chord_deviation_bound_mm(&self) -> Option<f64> {
        self.deviation_bound_mm
    }

    /// Work counters of the seal.
    pub const fn stats(&self) -> &crate::ops::OpStats {
        self.op.stats()
    }
}

/// A section reduced to placed points: the flat point list in profile order (outer
/// contour then cavities) with each contour's index range.
struct Section {
    points: Vec<Point2>,
    contours: Vec<(usize, usize)>,
    /// Largest distance of a section point from the section origin, millimetres.
    radius_mm: f64,
    /// Section centroid in section coordinates (signed-area weighted, cavities included).
    centroid: (f64, f64),
}

/// An orthonormal section frame kept as three axes; the initial frame is certified by
/// `Frame3`, and every later rotation moves each axis by a checked Rodrigues.
#[derive(Debug, Clone, Copy)]
struct Axes {
    x: Vec3,
    y: Vec3,
    z: Vec3,
}

/// The path reduced to chord vertices, with the true end tangents of an arc and the
/// achieved chord-deviation bound of an arc.
struct Chords {
    points: Vec<Point3>,
    start_tangent: Option<Vec3>,
    end_tangent: Option<Vec3>,
    deviation_bound_mm: Option<f64>,
}

/// The welded rings of a traversal: placed vertices, each ring's plane normal, the chord
/// lengths and directions, and each segment's parallel frame.
struct Walk {
    vertices: Vec<[f64; 3]>,
    normals: Vec<Vec3>,
    lengths: Vec<f64>,
    dirs: Vec<Vec3>,
    frames: Vec<Axes>,
}

fn numeric() -> SweepFailure {
    SweepFailure::Path(PathCode::NumericUncertain)
}

fn unit(v: Vec3) -> Result<Vec3, SweepFailure> {
    v.normalized().map_err(|_| numeric())
}

fn cross_norm(a: Vec3, b: Vec3) -> Result<f64, SweepFailure> {
    let cross = a.cross(b).map_err(|_| numeric())?;
    cross.norm().map_err(|_| numeric())
}

/// Rotates `v` about the unit axis `u` by the angle with the given cosine and sine
/// (Rodrigues, checked operations only).
fn rotated(v: Vec3, u: Vec3, cos: f64, sin: f64) -> Result<Vec3, SweepFailure> {
    let along = u
        .scaled(u.dot(v).map_err(|_| numeric())? * (1.0 - cos))
        .map_err(|_| numeric())?;
    let turn = v
        .scaled(cos)
        .map_err(|_| numeric())?
        .added(
            u.cross(v)
                .map_err(|_| numeric())?
                .scaled(sin)
                .map_err(|_| numeric())?,
        )
        .map_err(|_| numeric())?;
    turn.added(along).map_err(|_| numeric())
}

fn rotate_axes(axes: Axes, u: Vec3, cos: f64, sin: f64) -> Result<Axes, SweepFailure> {
    Ok(Axes {
        x: rotated(axes.x, u, cos, sin)?,
        y: rotated(axes.y, u, cos, sin)?,
        z: rotated(axes.z, u, cos, sin)?,
    })
}

/// The F06 default member frame over a unit chord: `Y` the projection of global `+Z`,
/// with the near-vertical `+Y` branch when the chord is within `PARALLEL_SQUARED` of
/// vertical (the written `MemberFrame` rule, rebuilt over a unit chord so no per-segment
/// axis floor applies to short arc facets).
fn default_frame(dir: Vec3) -> Result<Axes, SweepFailure> {
    let (dx, dy, dz) = (dir.x(), dir.y(), dir.z());
    let hh = dx * dx + dy * dy;
    let initial_y = if hh >= PARALLEL_SQUARED {
        let h = hh.sqrt();
        Vec3::new(-dx * dz / h, -dy * dz / h, hh / h).map_err(|_| numeric())?
    } else {
        Vec3::new(-dx * dy, 1.0 - dy * dy, -dz * dy)
            .map_err(|_| numeric())?
            .normalized()
            .map_err(|_| numeric())?
    };
    let x = initial_y
        .cross(dir)
        .map_err(|_| numeric())?
        .normalized()
        .map_err(|_| numeric())?;
    let y = dir.cross(x).map_err(|_| numeric())?;
    let frame = Frame3::try_new(x, y, dir).map_err(|_| numeric())?;
    Ok(Axes {
        x: frame.x(),
        y: frame.y(),
        z: frame.z(),
    })
}

/// Places one ring of section points **cut by the ring's plane**: every section point
/// rides its segment's fibre `origin + w + s*dir`, and the ring is where that fibre meets
/// the plane with the given normal through `origin` — `s = -(n*w)/(n*dir)`. For a
/// perpendicular end ring (`w` perpendicular to the normal) this is the plain rotated
/// section; at a mitre it is the affine oblique section both adjacent prisms share.
fn ring_points(
    origin: Point3,
    frame: &Axes,
    normal: Vec3,
    dir: Vec3,
    points: &[Point2],
    out: &mut Vec<[f64; 3]>,
) -> Result<(), SweepFailure> {
    let along = normal.dot(dir).map_err(|_| numeric())?;
    if along.abs() <= 1e-150 {
        return Err(numeric());
    }
    for point in points {
        let offset = frame
            .x
            .scaled(point.x)
            .map_err(|_| numeric())?
            .added(frame.y.scaled(point.y).map_err(|_| numeric())?)
            .map_err(|_| numeric())?;
        let cut = -(normal.dot(offset).map_err(|_| numeric())?) / along;
        let shift = dir.scaled(cut).map_err(|_| numeric())?;
        let placed = origin
            .translated(offset)
            .and_then(|p| p.translated(shift))
            .map_err(|_| numeric())?;
        out.push([placed.x, placed.y, placed.z]);
    }
    Ok(())
}

/// Validates one submitted path point (finiteness, then the coordinate cap).
fn checked_point(point: Point3) -> Result<(), SweepFailure> {
    point
        .validated()
        .map_err(|_| SweepFailure::Path(PathCode::NonFinite))?;
    if point.x.abs() > COORDINATE_CAP_MM
        || point.y.abs() > COORDINATE_CAP_MM
        || point.z.abs() > COORDINATE_CAP_MM
    {
        return Err(SweepFailure::Path(PathCode::AboveLimit));
    }
    Ok(())
}

fn checked_deviation(deviation: f64) -> Result<f64, SweepFailure> {
    if !deviation.is_finite() {
        return Err(SweepFailure::Path(PathCode::NonFinite));
    }
    if deviation <= 0.0 {
        return Err(SweepFailure::Path(PathCode::BadDeviation));
    }
    if deviation > DIMENSION_CAP_MM {
        return Err(SweepFailure::Path(PathCode::AboveLimit));
    }
    Ok(deviation)
}

impl Section {
    fn new(shape: &SectionOutline, profile: &Profile) -> Result<Self, SweepFailure> {
        let mut points = Vec::new();
        let mut contours = Vec::new();
        points.extend(shape.outer.iter().copied());
        contours.push((0, points.len()));
        for cavity in shape.cavities.iter() {
            let start = points.len();
            points.extend(cavity.points().iter().copied());
            contours.push((start, points.len()));
        }
        let mut radius_mm = 0.0_f64;
        for point in &points {
            let r = (point.x * point.x + point.y * point.y).sqrt();
            if !r.is_finite() {
                return Err(numeric());
            }
            radius_mm = radius_mm.max(r);
        }
        // Signed-area centroid over every contour; clockwise cavities carry negative
        // area, so the weighted sum is the centroid of the net region. Admission
        // guarantees a nonzero net area.
        let area = profile.net_area_mm2();
        if !area.is_finite() || area == 0.0 {
            return Err(numeric());
        }
        let (mut nx, mut ny) = (0.0_f64, 0.0_f64);
        for (start, end) in &contours {
            for i in *start..*end {
                let p = points.get(i).copied().ok_or_else(numeric)?;
                let q = points
                    .get(if i + 1 == *end { *start } else { i + 1 })
                    .copied()
                    .ok_or_else(numeric)?;
                let cross = p.x * q.y - q.x * p.y;
                nx += (p.x + q.x) * cross;
                ny += (p.y + q.y) * cross;
            }
        }
        let centroid = (nx / (6.0 * area), ny / (6.0 * area));
        if !centroid.0.is_finite() || !centroid.1.is_finite() {
            return Err(numeric());
        }
        Ok(Self {
            points,
            contours,
            radius_mm,
            centroid,
        })
    }
}

fn chord_polyline(points: &[Point3]) -> Result<Chords, SweepFailure> {
    if points.len() > MAX_PATH_POINTS {
        return Err(SweepFailure::Path(PathCode::AboveLimit));
    }
    if points.len() < 2 {
        return Err(SweepFailure::Path(PathCode::TooFewPoints));
    }
    for point in points {
        checked_point(*point)?;
    }
    Ok(Chords {
        points: points.to_vec(),
        start_tangent: None,
        end_tangent: None,
        deviation_bound_mm: None,
    })
}

/// The circumcircle of the three arc points, solved in the chord basis at `start`: with
/// `e1` the unit start-to-on chord of length `L` and `e2` the in-plane direction across
/// it, the centre sits at `L/2` along `e1` and at `((q2x^2 + q2y^2) - L*q2x) / (2*q2y)`
/// across it, where `q2` is the end point in the basis (`q2y` vanishes exactly when the
/// points are collinear). Facets: the G05 segment choice at the outermost fibre radius,
/// which is at most the arc radius plus twice the section radius (centroid within one
/// section radius of the path, surface one further section radius out).
fn chord_arc(
    start: Point3,
    on: Point3,
    end: Point3,
    deviation: f64,
    section_radius: f64,
) -> Result<Chords, SweepFailure> {
    for point in [start, on, end] {
        checked_point(point)?;
    }
    let chord_on = start.vector_to(on).map_err(|_| numeric())?;
    let chord_end = start.vector_to(end).map_err(|_| numeric())?;
    let length = chord_on.norm().map_err(|_| numeric())?;
    if length <= 0.0 {
        return Err(SweepFailure::Path(PathCode::CollinearArc));
    }
    let e1 = unit(chord_on)?;
    let plane_norm = cross_norm(chord_on, chord_end)?;
    if plane_norm <= 1e-150 {
        return Err(SweepFailure::Path(PathCode::CollinearArc));
    }
    let normal = chord_on.cross(chord_end).map_err(|_| numeric())?;
    let across_axis = normal
        .scaled(1.0 / plane_norm)
        .map_err(|_| numeric())?
        .cross(e1)
        .map_err(|_| numeric())?;
    let e2 = unit(across_axis)?;
    let q2x = chord_end.dot(e1).map_err(|_| numeric())?;
    let q2y = chord_end.dot(e2).map_err(|_| numeric())?;
    if q2y.abs() <= 1e-150 {
        return Err(SweepFailure::Path(PathCode::CollinearArc));
    }
    let across = ((q2x * q2x + q2y * q2y) - length * q2x) / (2.0 * q2y);
    if !across.is_finite() {
        return Err(SweepFailure::Path(PathCode::CollinearArc));
    }
    let centre = {
        let along = e1.scaled(length / 2.0).map_err(|_| numeric())?;
        let across_offset = e2.scaled(across).map_err(|_| numeric())?;
        start
            .translated(along)
            .and_then(|p| p.translated(across_offset))
            .map_err(|_| numeric())?
    };
    let radius = (length * length / 4.0 + across * across).sqrt();
    if !radius.is_finite() {
        return Err(SweepFailure::Path(PathCode::CollinearArc));
    }
    if radius > DIMENSION_CAP_MM {
        return Err(SweepFailure::Path(PathCode::AboveLimit));
    }
    // Angles in the (e1, e2) basis about the centre; travel passes through `on`. An end
    // equal to the start never reaches this point: its chord is the zero vector, so the
    // plane check above has already refused the call as coincident points.
    let a0 = (-length / 2.0).atan2(-across);
    let delta_on = ((length / 2.0).atan2(-across) - a0).rem_euclid(TAU);
    let delta_end = (((q2x - length / 2.0).atan2(q2y - across)) - a0).rem_euclid(TAU);
    let (sign, sweep) = if delta_on <= delta_end {
        (1.0, delta_end)
    } else {
        (-1.0, TAU - delta_end)
    };
    if !(sweep > 0.0 && sweep.is_finite()) {
        return Err(SweepFailure::Path(PathCode::CollinearArc));
    }
    let spec = CurveSpec {
        max_sagitta_mm: deviation,
        min_segments: 8,
        max_segments: 1020,
    };
    let radius_out = radius + 2.0 * section_radius;
    let (per_turn, _) = choose_segments(radius_out, spec)
        .map_err(|_| SweepFailure::Path(PathCode::DeviationUnreachable))?;
    let step = TAU / f64::from(per_turn);
    let facets = (sweep / step).ceil().max(1.0);
    if !facets.is_finite() || facets > 1021.0 {
        return Err(SweepFailure::Path(PathCode::DeviationUnreachable));
    }
    let facet_angle = sweep / facets;
    let tangent = |phi: f64| -> Result<Vec3, SweepFailure> {
        e1.scaled(-phi.sin() * sign)
            .map_err(|_| numeric())?
            .added(e2.scaled(phi.cos() * sign).map_err(|_| numeric())?)
            .map_err(|_| numeric())
    };
    let mut points = Vec::new();
    for j in 0..=facets as u64 {
        let phi = a0 + sign * facet_angle * j as f64;
        let offset = e1
            .scaled(radius * phi.cos())
            .map_err(|_| numeric())?
            .added(e2.scaled(radius * phi.sin()).map_err(|_| numeric())?)
            .map_err(|_| numeric())?;
        points.push(centre.translated(offset).map_err(|_| numeric())?);
    }
    Ok(Chords {
        points,
        start_tangent: Some(tangent(a0)?),
        end_tangent: Some(tangent(a0 + sign * sweep)?),
        deviation_bound_mm: Some(radius_out * facet_angle * facet_angle / 8.0),
    })
}

/// Traverses the chords: chord floors and the total-length cap, the certified default
/// frame on the first chord (twisted to the true start tangent), a mitre ring at every
/// interior vertex, and the end ring twisted to the true end tangent.
fn walk(path: &Chords, section: &Section, floor: f64) -> Result<Walk, SweepFailure> {
    let segments = path.points.len() - 1;
    let mut lengths = Vec::new();
    let mut dirs = Vec::new();
    let mut total = 0.0_f64;
    for j in 0..segments {
        let a = path.points.get(j).copied().ok_or_else(numeric)?;
        let b = path.points.get(j + 1).copied().ok_or_else(numeric)?;
        let delta = a.vector_to(b).map_err(|_| numeric())?;
        let len = delta.norm().map_err(|_| numeric())?;
        if len < floor {
            return Err(SweepFailure::Path(PathCode::ZeroLengthSegment));
        }
        total += len;
        if total > DIMENSION_CAP_MM {
            return Err(SweepFailure::Path(PathCode::AboveLimit));
        }
        lengths.push(len);
        dirs.push(unit(delta)?);
    }
    let mut vertices = Vec::new();
    let mut normals = Vec::new();
    let mut frames = Vec::new();
    let first_dir = dirs.first().copied().ok_or_else(numeric)?;
    let first = default_frame(first_dir)?;
    // The end rings are the parallel prism's fibres cut by the true end-tangent plane
    // (or the perpendicular end plane when no tangent exists) — the same oblique-section
    // construction as the mitres, so every segment stays an exact truncated prism.
    let start_normal = path.start_tangent.unwrap_or(first_dir);
    let origin = path.points.first().copied().ok_or_else(numeric)?;
    ring_points(
        origin,
        &first,
        start_normal,
        first_dir,
        &section.points,
        &mut vertices,
    )?;
    normals.push(start_normal);
    frames.push(first);
    let mut axes = first;
    for j in 1..segments {
        let d0 = dirs.get(j - 1).copied().ok_or_else(numeric)?;
        let d1 = dirs.get(j).copied().ok_or_else(numeric)?;
        let sin = cross_norm(d0, d1)?;
        if sin <= MIN_TURN_CROSS {
            return Err(SweepFailure::Path(PathCode::DegenerateTurn));
        }
        let cos = d0.dot(d1).map_err(|_| numeric())?;
        let u = d0
            .cross(d1)
            .map_err(|_| numeric())?
            .scaled(1.0 / sin)
            .map_err(|_| numeric())?;
        let mitre = rotate_axes(
            axes,
            u,
            (0.5 * (1.0 + cos).max(0.0)).sqrt(),
            (0.5 * (1.0 - cos).max(0.0)).sqrt(),
        )?;
        // The ring is the shared oblique section of both prisms: emitted from this
        // segment's parallel frame cut by the mitre plane (the identity that the same
        // cut seen from the previous segment reproduces the same points is exact).
        axes = rotate_axes(axes, u, cos, sin)?;
        let origin = path.points.get(j).copied().ok_or_else(numeric)?;
        ring_points(origin, &axes, mitre.z, d1, &section.points, &mut vertices)?;
        normals.push(mitre.z);
        frames.push(axes);
    }
    let last_dir = dirs.get(segments - 1).copied().ok_or_else(numeric)?;
    let end_normal = path.end_tangent.unwrap_or(last_dir);
    let origin = path.points.get(segments).copied().ok_or_else(numeric)?;
    ring_points(
        origin,
        &axes,
        end_normal,
        last_dir,
        &section.points,
        &mut vertices,
    )?;
    normals.push(end_normal);
    Ok(Walk {
        vertices,
        normals,
        lengths,
        dirs,
        frames,
    })
}

/// The analytic volume `A * sum(fibre lengths)`: the fibre of segment `j` runs through its
/// section centroid, and the cutting planes of both ends cross it at affine functions of
/// the section coordinates, so the truncated prism volume is the area times their
/// difference — exact in real arithmetic for any mitre angles.
fn analytic_volume(traversal: &Walk, section: &Section, area: f64) -> Result<f64, SweepFailure> {
    let (cx, cy) = section.centroid;
    let mut sum = 0.0_f64;
    let quads = traversal
        .lengths
        .iter()
        .zip(traversal.dirs.iter())
        .zip(traversal.frames.iter())
        .zip(traversal.normals.windows(2));
    for (((len, dir), frame), pair) in quads {
        let (start_normal, end_normal) = match (pair.first(), pair.last()) {
            (Some(a), Some(b)) => (*a, *b),
            _ => return Err(numeric()),
        };
        let offset = frame
            .x
            .scaled(cx)
            .map_err(|_| numeric())?
            .added(frame.y.scaled(cy).map_err(|_| numeric())?)
            .map_err(|_| numeric())?;
        let into = |n: Vec3| -> Result<f64, SweepFailure> {
            Ok(n.dot(offset).map_err(|_| numeric())? / n.dot(*dir).map_err(|_| numeric())?)
        };
        let s_start = -into(start_normal)?;
        let s_end = len - into(end_normal)?;
        sum += s_end - s_start;
    }
    if !sum.is_finite() {
        return Err(numeric());
    }
    Ok(area * sum)
}

/// Summed area vector of a wall quad (its two triangles, in mesh winding order).
pub(crate) fn quad_area_vector(
    vertices: &[[f64; 3]],
    a: usize,
    b: usize,
    c: usize,
    d: usize,
) -> Option<[f64; 3]> {
    let tri = |i: usize, j: usize, k: usize| -> Option<[f64; 3]> {
        let (p, q, r) = (vertices.get(i)?, vertices.get(j)?, vertices.get(k)?);
        let (u, w) = (
            [q[0] - p[0], q[1] - p[1], q[2] - p[2]],
            [r[0] - p[0], r[1] - p[1], r[2] - p[2]],
        );
        Some([
            u[1] * w[2] - u[2] * w[1],
            u[2] * w[0] - u[0] * w[2],
            u[0] * w[1] - u[1] * w[0],
        ])
    };
    let first = tri(a, b, c)?;
    let second = tri(a, c, d)?;
    let sum = [
        first[0] + second[0],
        first[1] + second[1],
        first[2] + second[2],
    ];
    let norm = (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt();
    if !norm.is_finite() || norm == 0.0 {
        return None;
    }
    Some([sum[0] / norm, sum[1] / norm, sum[2] / norm])
}

/// Builds the candidate: wall quads per section edge and segment (each a face with its
/// area-vector normal), then the two planar caps from the G05 triangulation (the start cap
/// reversed).
fn build_mesh(
    traversal: &Walk,
    section: &Section,
    caps: &[[u32; 3]],
) -> Result<Candidate, SweepFailure> {
    let points = section.points.len();
    let rings = traversal.normals.len();
    let mut triangles = Vec::new();
    let mut faces = Vec::new();
    let mut ids = Vec::new();
    let mut normals = Vec::new();
    let mut edge = 0_usize;
    for j in 0..rings - 1 {
        let (base_a, base_b) = (j * points, (j + 1) * points);
        for (start, end) in &section.contours {
            for i in *start..*end {
                let i2 = if i + 1 == *end { *start } else { i + 1 };
                let (ai, ai2) = (base_a + i, base_a + i2);
                let (bi, bi2) = (base_b + i, base_b + i2);
                let to_u32 = |v: usize| u32::try_from(v).map_err(|_| numeric());
                triangles.push([to_u32(ai)?, to_u32(ai2)?, to_u32(bi2)?]);
                triangles.push([to_u32(ai)?, to_u32(bi2)?, to_u32(bi)?]);
                let face = u16::try_from(ids.len()).map_err(|_| numeric())?;
                faces.push(face);
                faces.push(face);
                ids.push(format!("sweep:w{edge}.{j}"));
                normals.push(quad_area_vector(&traversal.vertices, ai, ai2, bi2, bi));
                edge += 1;
            }
        }
    }
    let cap_normal = |z: Vec3, sign: f64| -> Option<[f64; 3]> {
        Some([sign * z.x(), sign * z.y(), sign * z.z()])
    };
    let start_z = cap_normal(
        traversal.normals.first().copied().ok_or_else(numeric)?,
        -1.0,
    );
    let end_z = cap_normal(traversal.normals.last().copied().ok_or_else(numeric)?, 1.0);
    let last_base = (rings - 1) * points;
    let to_u32 = |v: usize| u32::try_from(v).map_err(|_| numeric());
    for cap in caps {
        let (a, b, c) = (
            cap.first().copied().ok_or_else(numeric)?,
            cap.get(1).copied().ok_or_else(numeric)?,
            cap.get(2).copied().ok_or_else(numeric)?,
        );
        let (a, b, c) = (to_usize(a)?, to_usize(b)?, to_usize(c)?);
        let face = u16::try_from(ids.len()).map_err(|_| numeric())?;
        // The start cap is the same triangulation wound the other way (its outward
        // normal points against the start tangent).
        triangles.push([to_u32(a)?, to_u32(c)?, to_u32(b)?]);
        faces.push(face);
        ids.push("sweep:f.cap0".to_owned());
        normals.push(start_z);
        let face = u16::try_from(ids.len()).map_err(|_| numeric())?;
        triangles.push([
            to_u32(last_base + a)?,
            to_u32(last_base + b)?,
            to_u32(last_base + c)?,
        ]);
        faces.push(face);
        ids.push("sweep:f.cap1".to_owned());
        normals.push(end_z);
    }
    Ok(Candidate {
        vertices: traversal.vertices.clone(),
        triangles,
        faces,
        face_ids: ids,
        normals,
    })
}

/// `u32` cap-triangulation index widened to `usize` for vertex access.
fn to_usize(v: u32) -> Result<usize, SweepFailure> {
    usize::try_from(v).map_err(|_| numeric())
}

/// Sweeps the section along the path and seals the certified result.
///
/// Stages: the section outline and its G05 admission, the path reduction (an arc is
/// faceted to the stated chord deviation here), the chord floors and caps, the mesh
/// budget, the mitred ring traversal, and the G09 seal — which certifies the closed shell
/// and cross-checks the mesh volume against the analytic truncated-prism volume.
pub fn sweep(input: &Sweep) -> Result<SweepResult, SweepFailure> {
    let shape = outline(&input.shape, input.representation).map_err(SweepFailure::Section)?;
    let profile = Profile::admit(&shape.outer, &shape.cavities).map_err(SweepFailure::Profile)?;
    let section = Section::new(&shape, &profile)?;
    let deviation = checked_deviation(input.chord_deviation_mm)?;
    let is_arc = matches!(input.path, SweepPath::Arc { .. });
    let chords = match &input.path {
        SweepPath::Polyline(points) => chord_polyline(points)?,
        SweepPath::Arc { start, on, end } => {
            chord_arc(*start, *on, *end, deviation, section.radius_mm)?
        }
    };
    let caps = profile.triangulate().map_err(SweepFailure::Profile)?;
    let rings = chords.points.len();
    let points = section.points.len();
    let vertices = u64::try_from(points)
        .map_err(|_| numeric())?
        .checked_mul(u64::try_from(rings).map_err(|_| numeric())?)
        .ok_or(SweepFailure::Path(PathCode::TooManyVertices))?;
    let triangles = vertices
        .saturating_sub(u64::try_from(points).map_err(|_| numeric())?)
        .saturating_mul(2)
        .checked_add(
            2u64.checked_mul(u64::try_from(caps.len()).map_err(|_| numeric())?)
                .ok_or(SweepFailure::Path(PathCode::TooManyVertices))?,
        )
        .ok_or(SweepFailure::Path(PathCode::TooManyVertices))?;
    if vertices > MAX_SWEEP_VERTICES || triangles > MAX_SWEEP_TRIANGLES {
        return Err(SweepFailure::Path(PathCode::TooManyVertices));
    }
    let traversal = walk(&chords, &section, profile.tolerance().eps_geom_mm())?;
    let predicted = analytic_volume(&traversal, &section, profile.net_area_mm2())?;
    let slack = 1e-9 * predicted.abs() + 1e-6;
    let expected = Enclosure::new(predicted - slack, predicted + slack).ok_or_else(numeric)?;
    let genus = u32::try_from(shape.cavities.len()).map_err(|_| numeric())?;
    let candidate = build_mesh(&traversal, &section, &caps)?;
    let expect = Expect {
        kind: OpKind::Sweep,
        euler: None,
        genus: Some(genus),
        analytic_volume: Some(expected),
        plane_deviation_mm: None,
        stats: Default::default(),
    };
    let op = seal(
        candidate,
        profile.tolerance(),
        &OpLimits::DEFAULT,
        expect,
        &mut |_| false,
    )
    .map_err(SweepFailure::Body)?;
    Ok(SweepResult {
        segments: u32::try_from(rings - 1).map_err(|_| numeric())?,
        op,
        net_area_mm2: profile.net_area_mm2(),
        approximated: shape.approximated || is_arc,
        deviation_bound_mm: chords.deviation_bound_mm,
    })
}
