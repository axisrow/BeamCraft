// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Ordered F04 plate contour admission and rectangular bodies in their contour frame.
//! Exact binary64 rational signs decide area, winding, topology and rectangle support;
//! the scaled distance band decides planarity admission without moving any input point.

use beamcraft_geom::tolerance::Tolerance;
use beamcraft_geom::{Frame3, MathError, Point3, Vec3};
use beamcraft_kernel::PlateResult;
use beamcraft_kernel::bigint::{Big, Overflow};
use beamcraft_kernel::exact::{min_exponent, scaled, triangle_cross};
use beamcraft_kernel::mesh::{PLATE_TRIANGLES, PlateMesh, encode, self_check};
use beamcraft_kernel::profile::Point2;
use beamcraft_kernel::profile::predicates::{SegRel, segment_relation};
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use core::cmp::Ordering;

fn invalid(message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::InvalidArgument, "contour_points_mm", message)
}

fn uncertain() -> CommandError {
    CommandError::new(
        ErrorCode::UnsupportedGeometry,
        "contour_points_mm",
        "contour numeric certificate failed",
    )
}

fn math(error: MathError) -> CommandError {
    let code = match error {
        MathError::NonFinite => ErrorCode::NonFiniteNumber,
        MathError::AboveLimit => ErrorCode::LimitExceeded,
        MathError::NumericUncertain | MathError::InvalidFrame => ErrorCode::UnsupportedGeometry,
        _ => ErrorCode::InvalidArgument,
    };
    CommandError::new(
        code,
        "contour_points_mm",
        "contour dimension or coordinate is invalid",
    )
}

/// Validated authored contour, independent of whether its solid can be evaluated.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlateContour {
    /// First submitted point, the body origin.
    pub origin: Point3,
    /// Certified X along the first edge, Z along the ordered area vector, Y = Z cross X.
    /// Absent for admitted near-planar input, which is not repaired into an exact body.
    pub frame: Option<Frame3>,
    /// Local scale and inclusive planarity band of the submitted contour.
    pub tolerance: Tolerance,
    /// True only when every point lies exactly in the ordered area-vector plane.
    pub exactly_planar: bool,
    /// Exact rectangular closure and orthogonality; dimensions are rounded edge lengths.
    pub rectangle_mm: Option<[f64; 2]>,
}

fn difference(a: [f64; 3], b: [f64; 3], exponent: i32) -> Result<[Big; 3], Overflow> {
    let [ax, ay, az] = a;
    let [bx, by, bz] = b;
    Ok([
        scaled(ax, exponent)?.sub(&scaled(bx, exponent)?)?,
        scaled(ay, exponent)?.sub(&scaled(by, exponent)?)?,
        scaled(az, exponent)?.sub(&scaled(bz, exponent)?)?,
    ])
}

fn dot(a: &[Big; 3], b: &[Big; 3]) -> Result<Big, Overflow> {
    let [ax, ay, az] = a;
    let [bx, by, bz] = b;
    ax.mul(bx)?.add(&ay.mul(by)?)?.add(&az.mul(bz)?)
}

fn rectangle(points: &[[f64; 3]], exponent: i32) -> Result<bool, Overflow> {
    let [a, b, c, d] = points else {
        return Ok(false);
    };
    let ab = difference(*b, *a, exponent)?;
    let dc = difference(*c, *d, exponent)?;
    let ad = difference(*d, *a, exponent)?;
    let bc = difference(*c, *b, exponent)?;
    for (left, right) in ab.iter().zip(&dc).chain(ad.iter().zip(&bc)) {
        if left.sub(right)?.sign() != 0 {
            return Ok(false);
        }
    }
    Ok(dot(&ab, &ad)?.sign() == 0)
}

/// Admits F04's ordered contour before deciding supported evaluation. No input is repaired.
/// Count and coordinate caps precede allocation; exact work is bounded by 1024 points.
pub fn admit(points: &[[f64; 3]], thickness: f64) -> Result<PlateContour, CommandError> {
    if points.len() > 1024 {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "contour_points_mm",
            "too many contour points",
        ));
    }
    if points.len() < 3 {
        return Err(invalid("contour needs at least three points"));
    }
    if !thickness.is_finite() || points.iter().flatten().any(|v| !v.is_finite()) {
        return Err(math(MathError::NonFinite));
    }
    if points.iter().flatten().any(|v| v.abs() > 1e9) || thickness > 1e6 {
        return Err(math(MathError::AboveLimit));
    }
    let raw: Vec<_> = points
        .iter()
        .map(|[x, y, z]| Point3::new(*x, *y, *z))
        .collect();
    let tolerance = Tolerance::for_plate(&raw).map_err(math)?;
    let mut edges = Vec::with_capacity(points.len());
    for (a, b) in raw.iter().zip(raw.iter().cycle().skip(1)) {
        let length = a.checked_distance_to(*b).map_err(math)?;
        if length > 1e6 {
            return Err(math(MathError::AboveLimit));
        }
        edges.push(length);
    }
    tolerance.check_dimension(thickness).map_err(|e| {
        let mut error = math(e);
        error.field = "thickness_mm";
        error
    })?;
    if edges.iter().any(|l| *l < tolerance.eps_geom_mm()) {
        return Err(invalid("contour edge is below the geometric floor"));
    }
    let first = *points.first().ok_or_else(uncertain)?;
    let origin = *raw.first().ok_or_else(uncertain)?;
    let exponent = min_exponent(points.iter().flatten().copied());
    let mut area = [Big::ZERO, Big::ZERO, Big::ZERO];
    for (a, b) in points.iter().zip(points.iter().cycle().skip(1)) {
        let cross = triangle_cross(exponent, first, *a, *b).map_err(|_| uncertain())?;
        for (sum, term) in area.iter_mut().zip(cross) {
            *sum = sum.add(&term).map_err(|_| uncertain())?;
        }
    }
    // Start at Z and replace only on a strict increase: ties prefer Z, then Y, then X.
    let mut dominant = 2usize;
    for k in [1usize, 0] {
        if area
            .get(k)
            .ok_or_else(uncertain)?
            .cmp_mag(area.get(dominant).ok_or_else(uncertain)?)
            == Ordering::Greater
        {
            dominant = k;
        }
    }
    let sign = area.get(dominant).ok_or_else(uncertain)?.sign();
    if sign == 0 {
        return Err(invalid("contour area vector is zero"));
    }
    let component = |v: &Big| {
        v.enclose(exponent * 2)
            .map(|n| n.midpoint())
            .ok_or_else(uncertain)
    };
    let [nx, ny, nz] = &area;
    let z = Vec3::new(component(nx)?, component(ny)?, component(nz)?)
        .and_then(Vec3::normalized)
        .map_err(|_| uncertain())?;
    let mut exactly_planar = true;
    for p in points {
        let delta = difference(*p, first, exponent).map_err(|_| uncertain())?;
        exactly_planar &= dot(&area, &delta).map_err(|_| uncertain())?.sign() == 0;
        let distance = origin
            .vector_to(Point3::new(p[0], p[1], p[2]))
            .and_then(|d| d.dot(z))
            .map_err(|_| uncertain())?
            .abs();
        if distance > tolerance.plane_band_mm() {
            return Err(invalid("contour is outside the planarity band"));
        }
    }
    if sign < 0 {
        return Err(invalid("contour winding is clockwise"));
    }
    let projected: Vec<_> = points
        .iter()
        .map(|p| {
            let [x, y] = beamcraft_kernel::exact::project(*p, dominant);
            Point2::new(x, y)
        })
        .collect();
    // Exact closed-segment tests, including adjacent overlap/backtracking and distant touches.
    for (i, (a, b)) in projected
        .iter()
        .zip(projected.iter().cycle().skip(1))
        .enumerate()
    {
        if a == b {
            return Err(invalid("contour projection has a zero edge"));
        }
        for (j, (c, d)) in projected
            .iter()
            .zip(projected.iter().cycle().skip(1))
            .enumerate()
            .skip(i.saturating_add(1))
        {
            let adjacent =
                j == i.saturating_add(1) || (i == 0 && j == points.len().saturating_sub(1));
            let relation = segment_relation(*a, *b, *c, *d).map_err(|_| uncertain())?;
            if (adjacent && relation != SegRel::Touch)
                || (!adjacent && relation != SegRel::Disjoint)
            {
                return Err(invalid("contour intersects, touches or backtracks"));
            }
        }
    }
    let next = raw.get(1).copied().ok_or_else(uncertain)?;
    let x = origin
        .vector_to(next)
        .and_then(Vec3::normalized)
        .map_err(|_| uncertain())?;
    let y = z.cross(x).map_err(|_| uncertain())?;
    // A near-planar admitted polygon need not have X perpendicular to N. Do not repair it
    // into a frame: its authored data are valid, but no exact body is available.
    let frame = if exactly_planar {
        Some(Frame3::try_new(x, y, z).map_err(|_| uncertain())?)
    } else {
        None
    };
    let rectangle_mm = if exactly_planar && rectangle(points, exponent).map_err(|_| uncertain())? {
        Some([
            *edges.first().ok_or_else(uncertain)?,
            *edges.last().ok_or_else(uncertain)?,
        ])
    } else {
        None
    };
    Ok(PlateContour {
        origin,
        frame,
        tolerance,
        exactly_planar,
        rectangle_mm,
    })
}

/// Evaluates an exactly rectangular contour with F06's canonical eight role-bit vertices.
/// All validation is repeated because callers and saved model data are untrusted.
pub fn rectangular_body(points: &[[f64; 3]], thickness: f64) -> Result<PlateResult, CommandError> {
    let contour = admit(points, thickness)?;
    let [width, length] = contour.rectangle_mm.ok_or_else(uncertain)?;
    let frame = contour.frame.ok_or_else(uncertain)?;
    let [p0, p1, _, p3] = points else {
        return Err(uncertain());
    };
    let e1 = Point3::new(p0[0], p0[1], p0[2])
        .vector_to(Point3::new(p1[0], p1[1], p1[2]))
        .map_err(|_| uncertain())?;
    let e2 = Point3::new(p0[0], p0[1], p0[2])
        .vector_to(Point3::new(p3[0], p3[1], p3[2]))
        .map_err(|_| uncertain())?;
    let e3 = frame.z().scaled(thickness).map_err(|_| uncertain())?;
    let mut vertices = [Point3::ORIGIN; 8];
    for (i, point) in vertices.iter_mut().enumerate() {
        *point = contour
            .origin
            .translated(e1.scaled(f64::from(i & 1 != 0)).map_err(|_| uncertain())?)
            .and_then(|p| p.translated(e2.scaled(f64::from(i & 2 != 0))?))
            .and_then(|p| p.translated(e3.scaled(f64::from(i & 4 != 0))?))
            .map_err(|_| uncertain())?;
    }
    let (x, y, z) = (frame.x(), frame.y(), frame.z());
    let normals = [
        x.scaled(-1.0).map_err(|_| uncertain())?,
        x,
        y.scaled(-1.0).map_err(|_| uncertain())?,
        y,
        z.scaled(-1.0).map_err(|_| uncertain())?,
        z,
    ];
    self_check(&vertices, &PLATE_TRIANGLES, &normals).map_err(|_| uncertain())?;
    let (blob, digest) = encode(&vertices).map_err(|_| uncertain())?;
    Ok(PlateResult {
        mesh: PlateMesh {
            face_ids: &beamcraft_kernel::feature_id::PLATE_FACES,
            vertices,
            normals,
            blob,
            digest,
        },
        volume_mm3: (width * length) * thickness,
        tolerance: contour.tolerance,
    })
}
