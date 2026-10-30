// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! G05: planar profiles with cavities, curve-approximation cavities, triangulation and
//! straight extrusion into a sealed closed mesh.
//!
//! A [`Profile`] is one outer contour plus zero or more cavity contours in the profile
//! plane (millimetres, right-handed, the plane normal is +Z). The outer contour must be
//! counter-clockwise and every cavity clockwise as seen from +Z, so the material lies to
//! the left of every directed boundary edge. Points are never reordered, merged, snapped
//! or repaired (F04 P-1, F06 section 4 rule 4).
//!
//! # Scope and provenance
//!
//! * Topology decisions (winding, intersection, touching, containment) are exact signs of
//!   the rational values of the submitted binary64 coordinates (F04 P-1).
//! * Floor decisions (minimum edge, minimum clearance, minimum area) use the F06 section 4
//!   local-size floor `eps_geom` from the G01 `Tolerance`; they never change a topology
//!   decision.
//! * Curve cavities (circle, slot, ellipse) are deterministic **polygon approximations**.
//!   F06 0.1.0 has no curve support: they are flagged ([`CurveApproximation`]) and their
//!   volume status is [`VolumeStatus::Approximated`]. A contract amendment is needed before
//!   an engine may treat them as exact curved features.
//! * Mathematics: shoelace area, divergence-theorem volume, Euler characteristic of a closed
//!   surface (`V - E + F = 2 - 2g`), ear clipping with hole bridging (Eberly, public
//!   description of the method; implemented here from the mathematics, not from code),
//!   Taylor series for sine and cosine, and the linear-interpolation error bound
//!   `h^2/8 * max|gamma''|` for chord sagitta.

mod admit;
mod curve;
mod exact;
mod extrude;
pub(crate) mod predicates;
mod triangulate;

pub mod diagnostic;

pub use curve::CurveSpec;
pub(crate) use curve::{SEGMENT_CAP, SEGMENT_FLOOR, choose_segments, unit_direction};
pub use extrude::{
    Extrusion, ExtrusionMesh, FaceRole, VolumeStatus, extrude, extrude_profile, extrude_with_cancel,
};

use crate::profile::diagnostic::ProfileFailure;
use beamcraft_geom::tolerance::Tolerance;

/// Largest number of points of one contour (F06 section 2).
pub const MAX_CONTOUR_POINTS: usize = 1024;
/// Largest total number of points of one profile: the F06 section 6 vertex cap (262,144)
/// holds two copies of every point in the extruded mesh.
pub const MAX_TOTAL_POINTS: usize = 131_072;
/// Largest number of cavities of one profile (G05 bound, below the F06 face cap).
pub const MAX_CAVITIES: usize = 256;

/// A finite point in the profile plane, in millimetres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point2 {
    /// Coordinate along the profile X axis.
    pub x: f64,
    /// Coordinate along the profile Y axis.
    pub y: f64,
}

impl Point2 {
    /// Creates a point without validating it; admission validates every coordinate.
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// Kind of curve a cavity polygon approximates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurveKind {
    /// A circle (round hole).
    Circle,
    /// An obround slot: two semicircles joined by straight sides (slotted hole).
    Slot,
    /// An ellipse (elliptical hole).
    Ellipse,
}

/// Provenance and error bound of a cavity generated as a polygon approximation of a curve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveApproximation {
    /// The approximated curve.
    pub kind: CurveKind,
    /// Number of polygon edges per full turn used for the curved parts.
    pub segments: u32,
    /// Requested bound on the chord sagitta, in millimetres.
    pub max_sagitta_mm: f64,
    /// Proven upper bound on the distance between the curve and the polygon, in millimetres,
    /// never above [`Self::max_sagitta_mm`].
    pub sagitta_bound_mm: f64,
}

/// One cavity contour, clockwise, with its optional curve-approximation flag.
#[derive(Debug, Clone, PartialEq)]
pub struct Cavity {
    points: Vec<Point2>,
    approximation: Option<CurveApproximation>,
}

impl Cavity {
    /// An authored polygonal cavity: every edge is its own planar wall face.
    pub fn polygon(points: Vec<Point2>) -> Self {
        Self {
            points,
            approximation: None,
        }
    }

    pub(crate) fn approximated(points: Vec<Point2>, approximation: CurveApproximation) -> Self {
        Self {
            points,
            approximation: Some(approximation),
        }
    }

    /// Cavity points in input order (clockwise).
    pub fn points(&self) -> &[Point2] {
        &self.points
    }

    /// The approximation flag, or `None` for an authored polygon.
    pub const fn approximation(&self) -> Option<&CurveApproximation> {
        self.approximation.as_ref()
    }
}

/// An admitted profile: validated once, immutable afterwards.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    outer: Vec<Point2>,
    cavities: Vec<Cavity>,
    tolerance: Tolerance,
}

impl Profile {
    /// Validates a profile in F06 stage order and returns it, or every failure of the
    /// first failing stage. Allocation is bounded by the caps checked first.
    pub fn admit(outer: &[Point2], cavities: &[Cavity]) -> Result<Self, ProfileFailure> {
        admit::admit(outer, cavities, None)
    }

    /// Admission with a minimum local-size policy supplied by the owner of the surrounding
    /// body (see the operations in `ops`); the larger floor applies.
    pub(crate) fn admit_with_floor(
        outer: &[Point2],
        cavities: &[Cavity],
        floor: Tolerance,
    ) -> Result<Self, ProfileFailure> {
        admit::admit_with_floor(outer, cavities, None, Some(floor))
    }

    pub(crate) fn from_parts(
        outer: Vec<Point2>,
        cavities: Vec<Cavity>,
        tolerance: Tolerance,
    ) -> Self {
        Self {
            outer,
            cavities,
            tolerance,
        }
    }

    /// The outer contour, counter-clockwise, in input order.
    pub fn outer(&self) -> &[Point2] {
        &self.outer
    }

    /// The cavities in input order.
    pub fn cavities(&self) -> &[Cavity] {
        &self.cavities
    }

    /// Local-size policy used for admission.
    pub const fn tolerance(&self) -> Tolerance {
        self.tolerance
    }

    /// Total number of contour points; the extruded mesh has twice as many vertices.
    pub fn point_count(&self) -> usize {
        self.outer.len() + self.cavities.iter().map(|c| c.points.len()).sum::<usize>()
    }

    /// Net area (outer minus cavities) in mm2, by the shoelace formula relative to each
    /// contour's first point, in the written operation order.
    pub fn net_area_mm2(&self) -> f64 {
        let (outer, _) = shoelace(&self.outer);
        self.cavities
            .iter()
            .fold(outer, |acc, c| acc + shoelace(&c.points).0)
    }

    /// Triangulates the cap: vertex indices run over the outer contour followed by each
    /// cavity in input order; every triangle is counter-clockwise (seen from +Z).
    pub fn triangulate(&self) -> Result<Vec<[u32; 3]>, ProfileFailure> {
        triangulate::triangulate(self)
    }
}

/// Point `i` of a closed contour with wrap-around; never panics.
pub(crate) fn at(points: &[Point2], i: usize) -> Point2 {
    match points.len() {
        0 => Point2::new(0.0, 0.0),
        n => points
            .get(i % n)
            .copied()
            .unwrap_or_else(|| Point2::new(0.0, 0.0)),
    }
}

/// Signed area `0.5 * sum((p_i - p_0) x (p_(i+1) - p_0))` and the sum of absolute terms.
pub(crate) fn shoelace(points: &[Point2]) -> (f64, f64) {
    let Some(origin) = points.first() else {
        return (0.0, 0.0);
    };
    let mut sum = 0.0;
    let mut magnitude = 0.0;
    for (p, q) in points.iter().skip(1).zip(points.iter().skip(2)) {
        let term = (p.x - origin.x) * (q.y - origin.y) - (p.y - origin.y) * (q.x - origin.x);
        sum += term;
        magnitude += term.abs();
    }
    (0.5 * sum, 0.5 * magnitude)
}
