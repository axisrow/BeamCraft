// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A07.1: outlines of the parametric section families and straight members of any section.
//!
//! [`outline`] turns an admitted `model::section::SectionShape` into a G05 profile (one
//! counter-clockwise outer contour and clockwise cavities) in the section plane, centred on the
//! centre of the section's bounding box: width along `X`, height along `Y`. Every family has
//! axis-aligned straight edges, so each corner is either sharp or a quarter arc whose centre is
//! `P - r*d1 + r*d2` (`d1`, `d2` the unit directions of the edges into and out of the sharp corner
//! `P`); concave root fillets and convex bends use the same rule, and the inner and outer bends
//! of a cold-formed section share their centre because the inner polygon is the outer one offset
//! by the thickness.
//!
//! Two representations come from one shape (owner decision of 2026-10-18):
//!
//! * [`Representation::Detailed`]: every radius is a quarter arc, approximated by a polygon with
//!   its vertices on the arc (the G05 deterministic trigonometry, [`detailed_arcs`] bound). The result
//!   is flagged approximated; F06 0.1.0 has no exact curves.
//! * [`Representation::Simplified`]: every radius is a sharp corner, and a circular hollow
//!   section is a coarse polygon. It is a lightweight stand-in, never used for quantities.
//!
//! [`section_member`] extrudes the outline along the member axis with G05 and maps it into the
//! F06 member frame (`geom::MemberFrame`, the same frame, roll and minima as
//! [`crate::straight_member`]), then certifies the closed single shell with G06 before sealing it.

use crate::diagnostic::{Failure, Field};
use crate::member::{StraightMember, locate};
use crate::ops::{Body, OpFailure, OpField};
use crate::profile::diagnostic::{ProfileCode, ProfileFailure};
use crate::profile::{Cavity, CurveSpec, Point2, extrude};
use crate::trimesh::TriMesh;
use beamcraft_geom::tolerance::Tolerance;
use beamcraft_geom::{MemberFrame, Point3, RigidTransform};
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::section::{CustomSection, ParametricSection, SectionFamily, SectionShape};

/// Which outline of a section to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Representation {
    /// Every radius as an approximated arc: the default geometry and the source of quantities.
    Detailed,
    /// Sharp corners and a coarse circle: a lightweight view stand-in, not exact.
    Simplified,
}

/// Smallest sagitta bound of the detailed outline, in millimetres.
pub const DETAILED_MIN_SAGITTA_MM: f64 = 0.25;
/// Sagitta bound of the detailed outline relative to the radius (0.5 percent).
pub const DETAILED_RELATIVE_SAGITTA: f64 = 0.005;

/// Arc approximation of the detailed outline for a radius `r`: a sagitta bound of
/// `max(0.25 mm, 0.005 * r)`, with 16 to 128 edges per full turn.
///
/// The relative part keeps every circle at or below 32 edges per turn (`2*pi*sqrt(1/(8*0.005))`
/// is below 32), so a large hollow section stays within the first-slice layer budgets (U01: 1024
/// items per array, 64 KiB per layer). Quantities that need catalogue accuracy use the declared
/// section numbers, not the polygon.
pub fn detailed_arcs(radius_mm: f64) -> CurveSpec {
    CurveSpec {
        max_sagitta_mm: DETAILED_MIN_SAGITTA_MM.max(DETAILED_RELATIVE_SAGITTA * radius_mm),
        min_segments: 16,
        max_segments: 128,
    }
}

/// Edges per full turn of a circular hollow section in the simplified outline.
pub const SIMPLIFIED_CIRCLE_SEGMENTS: u64 = 16;

/// A section outline in the section plane, centred on its bounding box.
#[derive(Debug, Clone, PartialEq)]
pub struct SectionOutline {
    /// Outer contour, counter-clockwise.
    pub outer: Vec<Point2>,
    /// Cavities, each clockwise.
    pub cavities: Vec<Cavity>,
    /// Bounding-box width along `X`, millimetres.
    pub width_mm: f64,
    /// Bounding-box height along `Y`, millimetres.
    pub height_mm: f64,
    /// Whether any curve of the section is replaced by a polygon (or dropped).
    pub approximated: bool,
}

/// Why a section could not be built: the failure of the stage that refused it.
#[derive(Debug, Clone, PartialEq)]
pub enum SectionFailure {
    /// The section values were refused before any point was generated.
    Shape(ErrorCode),
    /// The member axis, roll or frame was refused.
    Frame(Failure),
    /// G05 refused the outline or the extrusion.
    Profile(ProfileFailure),
    /// G06 refused the placed mesh.
    Body(OpFailure),
}

impl SectionFailure {
    /// The F04 code of the failure.
    pub fn f04_code(&self) -> ErrorCode {
        match self {
            Self::Shape(code) => *code,
            Self::Frame(f) => f
                .diagnostics()
                .next()
                .map_or(ErrorCode::UnsupportedGeometry, |d| d.code.f04_code()),
            Self::Profile(f) => f
                .diagnostics()
                .first()
                .map_or(ErrorCode::UnsupportedGeometry, |d| d.code.f04_code()),
            Self::Body(f) => f.code.f04_code(),
        }
    }
}

/// One corner of a sharp outline: its position and the radius of its arc (zero is sharp).
#[derive(Debug, Clone, Copy)]
struct Corner {
    x: f64,
    y: f64,
    r: f64,
}

const fn c(x: f64, y: f64, r: f64) -> Corner {
    Corner { x, y, r }
}

/// Unit axis direction from `a` to `b` of an axis-aligned edge, or `None` for a degenerate or
/// oblique edge (which no family generates).
fn axis(a: Corner, b: Corner) -> Option<(i8, i8)> {
    let sign = |v: f64| {
        if v > 0.0 {
            1
        } else if v < 0.0 {
            -1
        } else {
            0
        }
    };
    match (sign(b.x - a.x), sign(b.y - a.y)) {
        (0, 0) => None,
        (sx, 0) => Some((sx, 0)),
        (0, sy) => Some((0, sy)),
        _ => None,
    }
}

/// Quadrant index of a unit axis direction: `+X` 0, `+Y` 1, `-X` 2, `-Y` 3.
const fn quadrant(d: (i8, i8)) -> u64 {
    match d {
        (1, _) => 0,
        (_, 1) => 1,
        (-1, _) => 2,
        _ => 3,
    }
}

fn contour_failure() -> SectionFailure {
    SectionFailure::Shape(ErrorCode::InvalidArgument)
}

/// Traces a closed counter-clockwise (or clockwise) corner list into points, replacing every
/// corner of nonzero radius by a quarter arc of `n` edges per full turn (`n` a multiple of four).
fn trace(corners: &[Corner], n: u64) -> Result<Vec<Point2>, SectionFailure> {
    let len = corners.len();
    let mut out = Vec::with_capacity(len.saturating_mul(usize::try_from(n / 4 + 1).unwrap_or(1)));
    for i in 0..len {
        let at = |k: usize| corners.get(k % len).copied().ok_or_else(contour_failure);
        let (prev, p, next) = (at(i + len - 1)?, at(i)?, at(i + 1)?);
        if p.r == 0.0 {
            out.push(Point2::new(p.x, p.y));
            continue;
        }
        let d1 = axis(prev, p).ok_or_else(contour_failure)?;
        let d2 = axis(p, next).ok_or_else(contour_failure)?;
        let (d1x, d1y) = (f64::from(d1.0), f64::from(d1.1));
        let (d2x, d2y) = (f64::from(d2.0), f64::from(d2.1));
        let centre = (p.x - p.r * d1x + p.r * d2x, p.y - p.r * d1y + p.r * d2y);
        // The arc runs from direction -d2 to direction d1 about the centre: counter-clockwise
        // for a left turn, clockwise for a right turn.
        let left = d1x * d2y - d1y * d2x > 0.0;
        let quarter = n / 4;
        let start = quadrant((-d2.0, -d2.1)) * quarter;
        for step in 0..=quarter {
            let k = if left { start + step } else { start + n - step };
            let (cos, sin) = crate::profile::unit_direction(k, n);
            out.push(Point2::new(centre.0 + p.r * cos, centre.1 + p.r * sin));
        }
    }
    out.dedup();
    Ok(out)
}

/// Reverses a counter-clockwise ring to clockwise, keeping the first point first.
fn reversed(ccw: Vec<Point2>) -> Vec<Point2> {
    let mut out = Vec::with_capacity(ccw.len());
    out.extend(ccw.first().copied());
    out.extend(ccw.iter().skip(1).rev().copied());
    out
}

fn shift(corners: &mut [Corner], width: f64, height: f64) {
    let (dx, dy) = (width / 2.0, height / 2.0);
    for corner in corners {
        corner.x -= dx;
        corner.y -= dy;
    }
}

fn circle(radius: f64, n: u64) -> Vec<Point2> {
    (0..n)
        .map(|k| {
            let (cos, sin) = crate::profile::unit_direction(k, n);
            Point2::new(radius * cos, radius * sin)
        })
        .collect()
}

/// Edges per full turn for the largest radius of a section under [`detailed_arcs`].
fn segments_for(radius: f64) -> Result<u64, SectionFailure> {
    crate::profile::choose_segments(radius, detailed_arcs(radius))
        .map(|(n, _)| u64::from(n))
        .map_err(SectionFailure::Profile)
}

/// The sharp corner list (in bounding-box coordinates) and size of a parametric family; the last
/// parameter's radius is already chosen by the caller.
fn corners(section: &ParametricSection) -> Result<(Vec<Corner>, f64, f64), SectionFailure> {
    let list = match (section.family(), section.values()) {
        (SectionFamily::IShape, &[h, b, tw, tf, r]) => {
            let (l, rt) = ((b - tw) / 2.0, (b + tw) / 2.0);
            let list = vec![
                c(0.0, 0.0, 0.0),
                c(b, 0.0, 0.0),
                c(b, tf, 0.0),
                c(rt, tf, r),
                c(rt, h - tf, r),
                c(b, h - tf, 0.0),
                c(b, h, 0.0),
                c(0.0, h, 0.0),
                c(0.0, h - tf, 0.0),
                c(l, h - tf, r),
                c(l, tf, r),
                c(0.0, tf, 0.0),
            ];
            (list, b, h)
        }
        (SectionFamily::Channel, &[h, b, tw, tf, r]) => {
            let list = vec![
                c(0.0, 0.0, 0.0),
                c(b, 0.0, 0.0),
                c(b, tf, 0.0),
                c(tw, tf, r),
                c(tw, h - tf, r),
                c(b, h - tf, 0.0),
                c(b, h, 0.0),
                c(0.0, h, 0.0),
            ];
            (list, b, h)
        }
        (SectionFamily::Angle, &[h, b, t, r]) => {
            let list = vec![
                c(0.0, 0.0, 0.0),
                c(b, 0.0, 0.0),
                c(b, t, 0.0),
                c(t, t, r),
                c(t, h, 0.0),
                c(0.0, h, 0.0),
            ];
            (list, b, h)
        }
        (SectionFamily::Rhs, &[h, b, _, r]) => {
            let list = vec![c(0.0, 0.0, r), c(b, 0.0, r), c(b, h, r), c(0.0, h, r)];
            (list, b, h)
        }
        (SectionFamily::PlainC, &[h, b, t, r]) => {
            let big = if r > 0.0 { r + t } else { 0.0 };
            let list = vec![
                c(0.0, 0.0, big),
                c(b, 0.0, 0.0),
                c(b, t, 0.0),
                c(t, t, r),
                c(t, h - t, r),
                c(b, h - t, 0.0),
                c(b, h, 0.0),
                c(0.0, h, big),
            ];
            (list, b, h)
        }
        (SectionFamily::LippedC, &[h, b, lip, t, r]) => {
            let big = if r > 0.0 { r + t } else { 0.0 };
            let list = vec![
                c(0.0, 0.0, big),
                c(b, 0.0, big),
                c(b, lip, 0.0),
                c(b - t, lip, 0.0),
                c(b - t, t, r),
                c(t, t, r),
                c(t, h - t, r),
                c(b - t, h - t, r),
                c(b - t, h - lip, 0.0),
                c(b, h - lip, 0.0),
                c(b, h, big),
                c(0.0, h, big),
            ];
            (list, b, h)
        }
        (SectionFamily::LippedZ, &[h, b, lip, t, r]) => {
            let big = if r > 0.0 { r + t } else { 0.0 };
            let w = 2.0 * b - t;
            let list = vec![
                c(0.0, lip, 0.0),
                c(0.0, 0.0, big),
                c(b, 0.0, big),
                c(b, h - t, r),
                c(w - t, h - t, r),
                c(w - t, h - lip, 0.0),
                c(w, h - lip, 0.0),
                c(w, h, big),
                c(b - t, h, big),
                c(b - t, t, r),
                c(t, t, r),
                c(t, lip, 0.0),
            ];
            (list, w, h)
        }
        (SectionFamily::Omega, &[h, b, brim, t, r]) => {
            let big = if r > 0.0 { r + t } else { 0.0 };
            let w = b + 2.0 * brim - 2.0 * t;
            let list = vec![
                c(0.0, 0.0, 0.0),
                c(brim, 0.0, big),
                c(brim, h - t, r),
                c(b + brim - 2.0 * t, h - t, r),
                c(b + brim - 2.0 * t, 0.0, big),
                c(w, 0.0, 0.0),
                c(w, t, 0.0),
                c(b + brim - t, t, r),
                c(b + brim - t, h, big),
                c(brim - t, h, big),
                c(brim - t, t, r),
                c(0.0, t, 0.0),
            ];
            (list, w, h)
        }
        _ => return Err(contour_failure()),
    };
    Ok(list)
}

/// The outline of an admitted section shape in the requested representation.
///
/// The shape is admitted again here (the kernel never trusts a caller's admission), so a refused
/// value is [`SectionFailure::Shape`] with its F04 code. A custom outline keeps its authored point
/// order and shape, translated as a whole so that the centre of its outer contour's bounding box
/// is the origin, like every family (the stored definition is unchanged); G05 decides its winding
/// and simplicity when it is extruded.
pub fn outline(
    shape: &SectionShape,
    representation: Representation,
) -> Result<SectionOutline, SectionFailure> {
    shape
        .validate()
        .map_err(|e| SectionFailure::Shape(e.code))?;
    match shape {
        SectionShape::Custom(custom) => Ok(custom_outline(custom)),
        SectionShape::Parametric(section) => parametric_outline(section, representation),
    }
}

fn custom_outline(custom: &CustomSection) -> SectionOutline {
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for [x, y] in &custom.outer {
        lo = [lo[0].min(*x), lo[1].min(*y)];
        hi = [hi[0].max(*x), hi[1].max(*y)];
    }
    let centre = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
    let points = |contour: &[[f64; 2]]| {
        contour
            .iter()
            .map(|[x, y]| Point2::new(*x - centre[0], *y - centre[1]))
            .collect::<Vec<_>>()
    };
    SectionOutline {
        outer: points(&custom.outer),
        cavities: custom
            .cavities
            .iter()
            .map(|c| Cavity::polygon(points(c)))
            .collect(),
        width_mm: hi[0] - lo[0],
        height_mm: hi[1] - lo[1],
        approximated: false,
    }
}

fn parametric_outline(
    section: &ParametricSection,
    representation: Representation,
) -> Result<SectionOutline, SectionFailure> {
    parametric_outline_with(section, representation, None)
}

/// The outline with an optional fixed number of edges per full turn (a multiple of four), which
/// the tests use to compare the polygon with the exact area at an arbitrarily fine subdivision.
fn parametric_outline_with(
    section: &ParametricSection,
    representation: Representation,
    forced_segments: Option<u64>,
) -> Result<SectionOutline, SectionFailure> {
    let simplified = representation == Representation::Simplified;
    if let (SectionFamily::Chs, &[d, t]) = (section.family(), section.values()) {
        let n = if simplified {
            SIMPLIFIED_CIRCLE_SEGMENTS
        } else {
            match forced_segments {
                Some(n) => n,
                None => segments_for(d / 2.0)?,
            }
        };
        return Ok(SectionOutline {
            outer: circle(d / 2.0, n),
            cavities: vec![Cavity::polygon(reversed(circle(d / 2.0 - t, n)))],
            width_mm: d,
            height_mm: d,
            approximated: true,
        });
    }
    let used = if simplified {
        section.without_radius()
    } else {
        section.clone()
    };
    let radius = used.values().last().copied().unwrap_or(0.0);
    let (mut list, width, height) = corners(&used)?;
    shift(&mut list, width, height);
    // The outer bend of a cold-formed section is the largest radius; one edge count per section
    // keeps every arc on the same angular grid.
    let largest = list.iter().fold(0.0f64, |m, c| m.max(c.r));
    let n = if largest > 0.0 {
        match forced_segments {
            Some(n) => n,
            None => segments_for(largest)?,
        }
    } else {
        4
    };
    let outer = trace(&list, n)?;
    let mut cavities = Vec::new();
    if let (SectionFamily::Rhs, &[h, b, t, r]) = (used.family(), used.values()) {
        let inner_r = if r > t { r - t } else { 0.0 };
        let mut inner = vec![
            c(t, t, inner_r),
            c(b - t, t, inner_r),
            c(b - t, h - t, inner_r),
            c(t, h - t, inner_r),
        ];
        shift(&mut inner, width, height);
        cavities.push(Cavity::polygon(reversed(trace(&inner, n)?)));
    }
    Ok(SectionOutline {
        outer,
        cavities,
        width_mm: width,
        height_mm: height,
        approximated: radius > 0.0 || simplified,
    })
}

/// Raw input of a straight member of any section, element-local millimetres and radians.
#[derive(Debug, Clone, PartialEq)]
pub struct SectionMember {
    /// Axis start `p1`.
    pub start: Point3,
    /// Axis end `p2`.
    pub end: Point3,
    /// The section shape.
    pub shape: SectionShape,
    /// Which outline to extrude.
    pub representation: Representation,
    /// Explicit roll about `+Z_m`; absent is zero.
    pub roll_rad: Option<f64>,
}

/// A sealed straight member of any section.
#[derive(Debug, Clone)]
pub struct SectionResult {
    body: Body,
    blob: Vec<u8>,
    digest: ContentDigest,
    net_area_mm2: f64,
    axis_length_mm: f64,
    volume_mm3: f64,
    approximated: bool,
}

impl SectionResult {
    /// The certified body (vertices, triangles, face table and normals).
    pub const fn body(&self) -> &Body {
        &self.body
    }

    /// The exact BCMESH01 blob of the placed mesh.
    pub fn blob(&self) -> &[u8] {
        &self.blob
    }

    /// SHA-256 of the blob.
    pub const fn mesh_digest(&self) -> ContentDigest {
        self.digest
    }

    /// Net area of the extruded outline, mm2 (shoelace of the generated polygon). This is the
    /// area of the detailed polygon the mesh is built from, not the exact section area: it
    /// differs from [`beamcraft_model::section::SectionShape::exact_area_mm2`] by the arc
    /// approximation and is not used for quantities.
    pub const fn net_area_mm2(&self) -> f64 {
        self.net_area_mm2
    }

    /// The axis length the outline is extruded over, mm.
    pub const fn axis_length_mm(&self) -> f64 {
        self.axis_length_mm
    }

    /// Volume of the mesh `net_area * axis_length`, mm3 (the polygon, not the exact section).
    pub const fn volume_mm3(&self) -> f64 {
        self.volume_mm3
    }

    /// Whether the outline approximates (or, simplified, drops) a curve.
    pub const fn approximated(&self) -> bool {
        self.approximated
    }

    /// The member's F06 local-size policy.
    pub const fn tolerance(&self) -> Tolerance {
        self.body.tolerance()
    }
}

/// Builds a closed straight member of any admitted section, validating every submitted value.
///
/// Order: the section shape, then the member frame from the axis, the outline's bounding box and
/// the roll (the [`crate::straight_member`] rules and codes), then the G05 outline admission and
/// extrusion along the axis length, then the rigid placement `world = R*local + p1` and the G06
/// certification of one closed shell.
pub fn section_member(input: &SectionMember) -> Result<SectionResult, SectionFailure> {
    let shape = outline(&input.shape, input.representation)?;
    let frame_input = StraightMember {
        start: input.start,
        end: input.end,
        width_mm: shape.width_mm,
        height_mm: shape.height_mm,
        roll_rad: input.roll_rad,
    };
    let frame = MemberFrame::new(
        input.start,
        input.end,
        shape.width_mm,
        shape.height_mm,
        input.roll_rad,
    )
    .map_err(|error| {
        let (code, field, stage) = locate(&frame_input, error);
        SectionFailure::Frame(Failure::one(code, field, stage))
    })?;
    let length = frame.scale().axis_length_mm();
    let extrusion =
        extrude(&shape.outer, &shape.cavities, length).map_err(SectionFailure::Profile)?;
    let numeric = || {
        SectionFailure::Frame(Failure::one(
            crate::diagnostic::Code::NumericUncertain,
            Field::Mesh,
            crate::diagnostic::Stage::Build,
        ))
    };
    let placement =
        RigidTransform::from_body_frame(frame.frame(), input.start).map_err(|_| numeric())?;
    let local = TriMesh::from_blob(extrusion.mesh().blob())
        .map_err(|e| SectionFailure::Body(OpFailure::from_mesh(e, OpField::Body)))?;
    let placed = local
        .placed(&placement)
        .map_err(|e| SectionFailure::Body(OpFailure::from_mesh(e, OpField::Body)))?;
    let normals = extrusion
        .mesh()
        .face_normals()
        .iter()
        .map(|n| match n {
            Some(v) => placement
                .apply_vector(*v)
                .map(|w| Some([w.x(), w.y(), w.z()]))
                .map_err(|_| numeric()),
            None => Ok(None),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let body = Body::from_mesh(
        placed,
        extrusion.mesh().face_table().to_vec(),
        normals,
        Some(frame.scale().tolerance()),
    )
    .map_err(SectionFailure::Body)?;
    let blob = body.to_blob();
    let digest = Sha256::digest(&blob).map_err(|_| numeric())?;
    let net_area_mm2 = extrusion.net_area_mm2();
    let volume_mm3 = net_area_mm2 * length;
    if !volume_mm3.is_finite() || volume_mm3 <= 0.0 {
        return Err(SectionFailure::Profile(ProfileFailure::global(
            ProfileCode::NumericUncertain,
            crate::profile::diagnostic::ProfileField::Thickness,
            crate::diagnostic::Stage::Quantities,
        )));
    }
    Ok(SectionResult {
        body,
        blob,
        digest,
        net_area_mm2,
        axis_length_mm: length,
        volume_mm3,
        approximated: shape.approximated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn family(f: SectionFamily, values: &[f64]) -> ParametricSection {
        ParametricSection::new(f, values.to_vec()).unwrap()
    }

    fn signed(points: &[Point2]) -> f64 {
        let doubled: f64 = points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .map(|(p, q)| p.x * q.y - q.x * p.y)
            .sum();
        doubled / 2.0
    }

    fn polygon_area(o: &SectionOutline) -> f64 {
        o.cavities
            .iter()
            .fold(signed(&o.outer), |a, c| a + signed(c.points()))
    }

    fn samples() -> Vec<ParametricSection> {
        vec![
            family(SectionFamily::IShape, &[298.0, 149.0, 5.5, 8.0, 13.0]),
            family(SectionFamily::Channel, &[200.0, 80.0, 6.0, 10.0, 10.0]),
            family(SectionFamily::Angle, &[100.0, 75.0, 8.0, 10.0]),
            family(SectionFamily::Rhs, &[150.0, 100.0, 5.0, 10.0]),
            family(SectionFamily::Rhs, &[150.0, 100.0, 8.0, 6.0]),
            family(SectionFamily::Chs, &[114.0, 5.0]),
            family(SectionFamily::LippedC, &[150.0, 50.0, 15.0, 2.0, 3.0]),
            family(SectionFamily::PlainC, &[100.0, 40.0, 2.0, 3.0]),
            family(SectionFamily::LippedZ, &[200.0, 60.0, 18.0, 2.0, 3.0]),
            family(SectionFamily::Omega, &[40.0, 30.0, 20.0, 0.8, 1.0]),
        ]
    }

    #[test]
    fn a_finely_subdivided_polygon_converges_to_the_exact_area_of_every_family() {
        for section in samples() {
            let exact = section.exact_area_mm2();
            let at = |n: u64| {
                let o =
                    parametric_outline_with(&section, Representation::Detailed, Some(n)).unwrap();
                polygon_area(&o)
            };
            let (coarse, fine) = (at(256), at(16_384));
            assert!(
                (fine - exact).abs() <= 1e-7 * exact,
                "{:?}: polygon {fine} exact {exact}",
                section.family()
            );
            // The error shrinks with the square of the edge length (256 -> 16384 is 64 times).
            if (coarse - exact).abs() > 1e-9 * exact {
                assert!((coarse - exact).abs() > 100.0 * (fine - exact).abs());
            }
        }
    }

    #[test]
    fn a_sharp_outline_polygon_is_exactly_the_closed_form() {
        for section in samples() {
            let sharp = section.without_radius();
            let o = parametric_outline_with(&sharp, Representation::Detailed, None);
            if section.family() == SectionFamily::Chs {
                continue;
            }
            let area = polygon_area(&o.unwrap());
            let exact = sharp.exact_area_mm2();
            assert!(
                (area - exact).abs() <= 1e-12 * exact,
                "{:?}",
                section.family()
            );
        }
    }

    #[test]
    fn the_stored_detailed_polygon_overstates_a_root_fillet_and_the_exact_area_does_not() {
        // 30B1-like I section: the chord polygon of the four root fillets is larger than the arcs.
        let i = family(SectionFamily::IShape, &[298.0, 149.0, 5.5, 8.0, 13.0]);
        let polygon = polygon_area(&parametric_outline(&i, Representation::Detailed).unwrap());
        let exact = i.exact_area_mm2();
        assert!(polygon > exact);
        assert!((polygon - exact) / exact < 0.0035);
        let closed_form = 3935.0 + (4.0 - core::f64::consts::PI) * 169.0;
        assert!((exact - closed_form).abs() <= 1e-9 * closed_form);
    }
}
