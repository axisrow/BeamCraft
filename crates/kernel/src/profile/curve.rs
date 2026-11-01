// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Deterministic polygon approximations of round, slotted and elliptical cavities.
//!
//! F06 0.1.0 defines no curved geometry, so every generator returns a clockwise polygon
//! flagged with [`CurveApproximation`]; nothing here is an exact curve. The polygon vertices lie
//! **on** the curve. The achieved bound is the linear-interpolation error of a C2 curve over
//! a parameter step `h = 2*pi/n`: the distance from the curve to its chord is at most
//! `h^2/8 * max|gamma''|` (for a circle, the radius; for an ellipse, its larger semi-axis).
//! The bound excludes the binary64 rounding of the vertex coordinates (a few ulps).
//!
//! Sine and cosine use only `+ - * /` (correctly rounded under IEEE 754), a symmetric
//! octant reduction by exact integer arithmetic and a fixed-length Taylor series, so the
//! output is bit-identical on every target and exactly symmetric about the axes.

use super::diagnostic::{Defect, ProfileCode, ProfileFailure, ProfileField, Stage};
use super::{Cavity, CurveApproximation, CurveKind, Point2};
use beamcraft_geom::tolerance::{COORDINATE_CAP_MM, DIMENSION_CAP_MM};
use core::f64::consts::{FRAC_PI_2, TAU};

/// Hard cap on polygon edges per full turn (a slot adds two points: 1022 < 1024).
pub const SEGMENT_CAP: u32 = 1020;
/// Smallest polygon edge count per full turn (a multiple of four, for exact symmetry).
pub const SEGMENT_FLOOR: u32 = 8;

/// Approximation request: a chord-sagitta bound and a segment count range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveSpec {
    /// Largest accepted distance between the curve and its polygon, in millimetres.
    pub max_sagitta_mm: f64,
    /// Fewest edges per full turn; rounded up to a multiple of four, at least 8.
    pub min_segments: u32,
    /// Most edges per full turn; rounded down to a multiple of four, at most 1020.
    pub max_segments: u32,
}

impl CurveSpec {
    /// 0.01 mm sagitta, between 16 and 512 edges per full turn.
    pub const DEFAULT: Self = Self {
        max_sagitta_mm: 0.01,
        min_segments: 16,
        max_segments: 512,
    };
}

impl Default for CurveSpec {
    fn default() -> Self {
        Self::DEFAULT
    }
}

fn fail(code: ProfileCode, defect: Defect) -> ProfileFailure {
    ProfileFailure::one(code, ProfileField::Curve, Stage::Ranges, defect, None, None)
}

fn positive_dimension(value: f64) -> Result<f64, ProfileFailure> {
    if !value.is_finite() {
        Err(fail(ProfileCode::NonFinite, Defect::CurveInput))
    } else if value > DIMENSION_CAP_MM {
        Err(fail(ProfileCode::AboveLimit, Defect::CurveInput))
    } else if value <= 0.0 {
        Err(fail(ProfileCode::NotPositive, Defect::CurveInput))
    } else {
        Ok(value)
    }
}

fn centre(point: Point2) -> Result<Point2, ProfileFailure> {
    if !(point.x.is_finite() && point.y.is_finite()) {
        Err(fail(ProfileCode::NonFinite, Defect::CurveInput))
    } else if point.x.abs() > COORDINATE_CAP_MM || point.y.abs() > COORDINATE_CAP_MM {
        Err(fail(ProfileCode::AboveLimit, Defect::CurveInput))
    } else {
        Ok(point)
    }
}

/// Unit vector along the submitted axis, by correctly rounded division by its norm.
fn unit_axis(axis: Point2) -> Result<Point2, ProfileFailure> {
    if !(axis.x.is_finite() && axis.y.is_finite()) {
        return Err(fail(ProfileCode::NonFinite, Defect::CurveInput));
    }
    if axis.x.abs() > COORDINATE_CAP_MM || axis.y.abs() > COORDINATE_CAP_MM {
        return Err(fail(ProfileCode::AboveLimit, Defect::CurveInput));
    }
    let norm = (axis.x * axis.x + axis.y * axis.y).sqrt();
    // Below 1e-150 the squares leave the normal range and the norm loses its precision.
    if norm < 1e-150 {
        return Err(fail(ProfileCode::NotPositive, Defect::CurveInput));
    }
    Ok(Point2::new(axis.x / norm, axis.y / norm))
}

fn round_up_to_four(n: u32) -> u32 {
    n.saturating_add(3) / 4 * 4
}

/// Smallest admissible edge count per full turn whose sagitta bound meets the request.
pub fn choose_segments(
    curvature_radius: f64,
    spec: CurveSpec,
) -> Result<(u32, f64), ProfileFailure> {
    if !spec.max_sagitta_mm.is_finite() {
        return Err(fail(ProfileCode::NonFinite, Defect::CurveInput));
    }
    if spec.max_sagitta_mm <= 0.0 {
        return Err(fail(ProfileCode::NotPositive, Defect::CurveInput));
    }
    if spec.max_segments > SEGMENT_CAP {
        return Err(fail(ProfileCode::AboveLimit, Defect::CurveInput));
    }
    let low = round_up_to_four(spec.min_segments.max(SEGMENT_FLOOR));
    let high = spec.max_segments / 4 * 4;
    if low > high {
        return Err(fail(ProfileCode::ContourInvalid, Defect::CurveInput));
    }
    let mut n = low;
    while n <= high {
        let step = TAU / f64::from(n);
        let bound = curvature_radius * step * step / 8.0;
        if bound <= spec.max_sagitta_mm {
            return Ok((n, bound));
        }
        n += 4;
    }
    Err(fail(ProfileCode::AboveLimit, Defect::SagittaUnreachable))
}

/// `(cos, sin)` of `psi = (num/den) * pi/2` for `0 <= psi <= pi/4`, by a 10-term Taylor series.
fn taylor(num: u64, den: u64) -> (f64, f64) {
    let psi = (num as f64 * FRAC_PI_2) / den as f64;
    let x2 = psi * psi;
    let (mut sin_acc, mut cos_acc) = (1.0, 1.0);
    for k in (1..=10u32).rev() {
        let (a, b) = (f64::from(2 * k), f64::from(2 * k + 1));
        sin_acc = 1.0 - x2 * sin_acc / (a * b);
        cos_acc = 1.0 - x2 * cos_acc / ((a - 1.0) * a);
    }
    (cos_acc, psi * sin_acc)
}

/// `(cos, sin)` of the angle `2*pi*k/n`, exactly symmetric under the dihedral group of the square.
pub fn unit_direction(k: u64, n: u64) -> (f64, f64) {
    if n == 0 {
        return (1.0, 0.0);
    }
    let k = k % n;
    let quadrant = (4 * k) / n;
    let num = 4 * k - quadrant * n;
    let (c, s) = if 2 * num > n {
        let (c, s) = taylor(n - num, n);
        (s, c)
    } else {
        taylor(num, n)
    };
    let (c, s) = match quadrant {
        0 => (c, s),
        1 => (-s, c),
        2 => (-c, -s),
        _ => (s, -c),
    };
    (c + 0.0, s + 0.0)
}

/// Point of an ellipse (or circle) with semi-axes `a` along `u` and `b` along `perp(u)`.
fn ellipse_point(centre: Point2, a: f64, b: f64, u: Point2, c: f64, s: f64) -> Point2 {
    let (along, across) = (a * c, b * s);
    Point2::new(
        centre.x + (along * u.x + across * (-u.y)),
        centre.y + (along * u.y + across * u.x),
    )
}

/// Reverses a counter-clockwise ring to clockwise while keeping the first point first.
fn clockwise(ccw: Vec<Point2>) -> Vec<Point2> {
    let mut out = Vec::with_capacity(ccw.len());
    out.extend(ccw.first().copied());
    out.extend(ccw.iter().skip(1).rev().copied());
    out
}

fn ring(centre: Point2, a: f64, b: f64, u: Point2, n: u32) -> Vec<Point2> {
    (0..u64::from(n))
        .map(|k| {
            let (c, s) = unit_direction(k, u64::from(n));
            ellipse_point(centre, a, b, u, c, s)
        })
        .collect()
}

impl Cavity {
    /// A round hole: a clockwise polygon with vertices on the circle.
    ///
    /// The first vertex is at angle 0 (the +X side). The edge count per full turn is a
    /// multiple of four in the requested range; the achieved sagitta bound is recorded.
    pub fn circle(
        centre_mm: Point2,
        radius_mm: f64,
        spec: CurveSpec,
    ) -> Result<Self, ProfileFailure> {
        let c = centre(centre_mm)?;
        let r = positive_dimension(radius_mm)?;
        let (n, bound) = choose_segments(r, spec)?;
        let points = clockwise(ring(c, r, r, Point2::new(1.0, 0.0), n));
        Ok(Self::approximated(
            points,
            CurveApproximation {
                kind: CurveKind::Circle,
                segments: n,
                max_sagitta_mm: spec.max_sagitta_mm,
                sagitta_bound_mm: bound,
            },
        ))
    }

    /// An elliptical hole with semi-axis `semi_u_mm` along `axis` and `semi_v_mm` across it.
    ///
    /// `axis` is any nonzero vector; it is normalised, never rotated by an angle function.
    pub fn ellipse(
        centre_mm: Point2,
        semi_u_mm: f64,
        semi_v_mm: f64,
        axis: Point2,
        spec: CurveSpec,
    ) -> Result<Self, ProfileFailure> {
        let c = centre(centre_mm)?;
        let a = positive_dimension(semi_u_mm)?;
        let b = positive_dimension(semi_v_mm)?;
        let u = unit_axis(axis)?;
        let (n, bound) = choose_segments(a.max(b), spec)?;
        Ok(Self::approximated(
            clockwise(ring(c, a, b, u, n)),
            CurveApproximation {
                kind: CurveKind::Ellipse,
                segments: n,
                max_sagitta_mm: spec.max_sagitta_mm,
                sagitta_bound_mm: bound,
            },
        ))
    }

    /// A slotted (obround) hole of overall `length_mm` along `axis` and `width_mm` across it:
    /// two semicircle ends joined by two straight sides. `length_mm` must exceed `width_mm`.
    pub fn slot(
        centre_mm: Point2,
        length_mm: f64,
        width_mm: f64,
        axis: Point2,
        spec: CurveSpec,
    ) -> Result<Self, ProfileFailure> {
        let c = centre(centre_mm)?;
        let length = positive_dimension(length_mm)?;
        let width = positive_dimension(width_mm)?;
        if length <= width {
            return Err(fail(ProfileCode::NotPositive, Defect::CurveInput));
        }
        let u = unit_axis(axis)?;
        let radius = width / 2.0;
        let half_straight = (length - width) / 2.0;
        let (n, bound) = choose_segments(radius, spec)?;
        let n_wide = u64::from(n);
        let quarter = n_wide / 4;
        let (right, left) = (
            Point2::new(c.x + half_straight * u.x, c.y + half_straight * u.y),
            Point2::new(c.x - half_straight * u.x, c.y - half_straight * u.y),
        );
        let arc = |centre: Point2, first: u64| {
            (first..=first + 2 * quarter).map(move |k| {
                let (cs, sn) = unit_direction(k, n_wide);
                ellipse_point(centre, radius, radius, u, cs, sn)
            })
        };
        // Counter-clockwise: right cap from -90 to +90 degrees, then left cap from 90 to 270.
        let mut ccw: Vec<Point2> = arc(right, n_wide - quarter).collect();
        ccw.extend(arc(left, quarter));
        Ok(Self::approximated(
            clockwise(ccw),
            CurveApproximation {
                kind: CurveKind::Slot,
                segments: n,
                max_sagitta_mm: spec.max_sagitta_mm,
                sagitta_bound_mm: bound,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_trigonometry_matches_the_standard_library_closely() {
        for n in [8u64, 12, 20, 36, 100, 1020] {
            for k in 0..n {
                let (c, s) = unit_direction(k, n);
                let angle = std::f64::consts::TAU * k as f64 / n as f64;
                assert!((c - angle.cos()).abs() < 5e-15, "cos n={n} k={k}");
                assert!((s - angle.sin()).abs() < 5e-15, "sin n={n} k={k}");
            }
        }
        assert_eq!(unit_direction(0, 8), (1.0, 0.0));
        assert_eq!(unit_direction(2, 8), (0.0, 1.0));
        assert_eq!(unit_direction(4, 8), (-1.0, 0.0));
        assert_eq!(unit_direction(6, 8), (0.0, -1.0));
    }

    #[test]
    fn symmetric_points_are_bit_identical() {
        let n = 36u64;
        for k in 0..n {
            let (c, s) = unit_direction(k, n);
            let (cm, sm) = unit_direction(n - k, n);
            assert_eq!(c.to_bits(), cm.to_bits());
            assert_eq!(s, -sm);
        }
    }

    #[test]
    fn segment_choice_honours_range_and_bound() {
        let spec = CurveSpec {
            max_sagitta_mm: 0.01,
            min_segments: 8,
            max_segments: 1020,
        };
        let (n, bound) = choose_segments(10.0, spec).unwrap();
        assert!(bound <= 0.01 && n % 4 == 0);
        let (smaller, _) = choose_segments(
            10.0,
            CurveSpec {
                min_segments: 8,
                ..spec
            },
        )
        .unwrap();
        assert_eq!(smaller, n);
        // One step fewer would miss the bound.
        let prev = TAU / f64::from(n - 4);
        assert!(10.0 * prev * prev / 8.0 > 0.01);
        assert!(
            choose_segments(
                1.0e6,
                CurveSpec {
                    max_segments: 64,
                    ..spec
                }
            )
            .is_err()
        );
    }
}
