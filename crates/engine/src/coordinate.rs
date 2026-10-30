// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T04 shared numeric point input. Distances are millimetres; polar angles are radians.

use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;

/// Direction constraint through the reference point, in model axes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AxisLock {
    /// All three coordinates can change.
    #[default]
    None,
    /// Only X can change.
    X,
    /// Only Y can change.
    Y,
    /// Only Z can change.
    Z,
}

fn non_finite() -> CommandError {
    CommandError::new(
        ErrorCode::NonFiniteNumber,
        "coordinate",
        "coordinates must be finite",
    )
}

fn invalid() -> CommandError {
    CommandError::new(
        ErrorCode::InvalidArgument,
        "coordinate",
        "expected X,Y,Z, @DX,DY,DZ or @DISTANCE<ANGLE[,DZ]; finite millimetres and radians",
    )
}

impl AxisLock {
    /// Parse a CLI/UI lock token.
    pub fn parse(text: &str) -> Result<Self, CommandError> {
        match text {
            "none" => Ok(Self::None),
            "x" => Ok(Self::X),
            "y" => Ok(Self::Y),
            "z" => Ok(Self::Z),
            _ => Err(invalid()),
        }
    }

    /// Stable CLI token.
    pub const fn token(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::X => "x",
            Self::Y => "y",
            Self::Z => "z",
        }
    }

    /// Project a point onto the locked axis through `reference`.
    pub fn constrain(self, reference: [f64; 3], point: [f64; 3]) -> Result<[f64; 3], CommandError> {
        if !reference.into_iter().chain(point).all(f64::is_finite) {
            return Err(non_finite());
        }
        let [x, y, z] = point;
        let [rx, ry, rz] = reference;
        Ok(match self {
            Self::None => [x, y, z],
            Self::X => [x, ry, rz],
            Self::Y => [rx, y, rz],
            Self::Z => [rx, ry, z],
        })
    }
}

/// Resolve absolute Cartesian, relative Cartesian, or relative XY polar input.
///
/// Polar input is `@distance<angle` with optional `,dz`. The reference supplies Z and the
/// origin; angles increase from +X towards +Y. A lock is applied after resolving the point.
/// Input length and field count are bounded before parsing; non-finite results are refused.
pub fn resolve(text: &str, reference: [f64; 3], lock: AxisLock) -> Result<[f64; 3], CommandError> {
    if text.len() > 128 || !reference.into_iter().all(f64::is_finite) {
        return Err(invalid());
    }
    let text = text.trim();
    let number = |s: &str| -> Result<f64, CommandError> {
        let n = s.trim().parse::<f64>().map_err(|_| invalid())?;
        if n.is_finite() {
            Ok(n)
        } else {
            Err(non_finite())
        }
    };
    let relative = text.strip_prefix('@');
    let coordinates = relative.unwrap_or(text);
    let point = if let Some((distance, rest)) = coordinates.split_once('<') {
        if relative.is_none() {
            return Err(invalid());
        }
        let (angle, dz) = rest.split_once(',').unwrap_or((rest, "0"));
        let distance = number(distance)?;
        if distance < 0.0 {
            return Err(invalid());
        }
        let angle = number(angle)?;
        [distance * angle.cos(), distance * angle.sin(), number(dz)?]
    } else {
        let mut fields = coordinates.split(',');
        let xyz = [
            number(fields.next().ok_or_else(invalid)?)?,
            number(fields.next().ok_or_else(invalid)?)?,
            number(fields.next().ok_or_else(invalid)?)?,
        ];
        if fields.next().is_some() {
            return Err(invalid());
        }
        xyz
    };
    let point = if relative.is_some() {
        let [x, y, z] = point;
        let [rx, ry, rz] = reference;
        [rx + x, ry + y, rz + z]
    } else {
        point
    };
    lock.constrain(reference, point)
}

/// Resolve a construction step toward a 3D cursor point. An optional length replaces the
/// cursor distance; orthogonal mode chooses the dominant X/Y/Z component in `frame`.
pub fn construction_step(
    reference: [f64; 3],
    toward: [f64; 3],
    distance_mm: Option<f64>,
    orthogonal: bool,
    frame: Option<crate::work_plane::WorkPlane>,
) -> Result<[f64; 3], CommandError> {
    use beamcraft_model::limits::COORDINATE_CAP_MM;
    let frame = frame.unwrap_or_default();
    let a = frame.to_local(reference)?;
    let b = frame.to_local(toward)?;
    let [ax, ay, az] = a;
    let [bx, by, bz] = b;
    let mut delta = [bx - ax, by - ay, bz - az];
    if orthogonal {
        let dominant = delta
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.abs().total_cmp(&b.abs()))
            .map(|(i, _)| i)
            .ok_or_else(invalid)?;
        for (axis, value) in delta.iter_mut().enumerate() {
            if axis != dominant {
                *value = 0.0;
            }
        }
    }
    if let Some(distance) = distance_mm {
        if !distance.is_finite() {
            return Err(non_finite());
        }
        if distance <= 0.0 || distance > COORDINATE_CAP_MM {
            return Err(invalid());
        }
        let length = delta.iter().map(|n| n * n).sum::<f64>().sqrt();
        if !length.is_finite() || length < 1e-9 {
            return Err(invalid());
        }
        for value in &mut delta {
            *value = *value / length * distance;
        }
    }
    let [dx, dy, dz] = delta;
    frame.to_world([ax + dx, ay + dy, az + dz])
}

/// Admit the plane of an ordered spatial contour. Keep the preferred frame when coplanar;
/// otherwise construct a frame from the first non-collinear ordered points. Never flatten a point.
pub fn contour_frame(
    points: &[[f64; 3]],
    preferred: Option<crate::work_plane::WorkPlane>,
) -> Result<crate::work_plane::WorkPlane, CommandError> {
    use crate::work_plane::WorkPlane;
    if !(3..=beamcraft_model::limits::CONTOUR_POINT_CAP).contains(&points.len()) {
        return Err(invalid());
    }
    let preferred = preferred.unwrap_or_default();
    let in_plane = |frame: WorkPlane| {
        points
            .iter()
            .all(|p| frame.to_local(*p).is_ok_and(|[_, _, z]| z.abs() <= 1e-6))
    };
    // Validate every point before deciding whether an earlier triple defines the frame.
    for point in points {
        preferred.to_local(*point)?;
    }
    if in_plane(preferred) {
        return Ok(preferred);
    }
    let a = *points.first().ok_or_else(invalid)?;
    let b = points
        .iter()
        .copied()
        .find(|b| a.iter().zip(b).any(|(a, b)| (a - b).abs() > 1e-9))
        .ok_or_else(invalid)?;
    for c in points {
        if let Ok(mut frame) = WorkPlane::three_points(a, b, *c) {
            // Inferred thickness direction must not flip when the designer walks the same
            // spatial outline in the opposite direction. Explicit session frames keep their normal.
            if frame
                .normal()
                .into_iter()
                .max_by(|a, b| a.abs().total_cmp(&b.abs()))
                .is_some_and(|n| n < 0.0)
            {
                frame = WorkPlane::new(frame.origin(), frame.x(), frame.y().map(|v| -v))?;
            }
            return if in_plane(frame) {
                Ok(frame)
            } else {
                Err(CommandError::new(
                    ErrorCode::InvalidArgument,
                    "contour",
                    "plate vertices must be coplanar",
                ))
            };
        }
    }
    Err(invalid())
}

/// Normalize a picked local XY contour to counter-clockwise order, keeping the first point.
/// Only traversal changes: topology and dimensional admission still belong to plate evaluation.
pub fn normalize_contour_winding(points: &[[f64; 3]]) -> Result<Vec<[f64; 3]>, CommandError> {
    if !(3..=beamcraft_model::limits::CONTOUR_POINT_CAP).contains(&points.len()) {
        return Err(invalid());
    }
    for point in points {
        crate::work_plane::WorkPlane::default().to_local(*point)?;
        if point.get(2).copied().unwrap_or(0.0).abs() > 1e-6 {
            return Err(invalid());
        }
    }
    let [ox, oy, _] = *points.first().ok_or_else(invalid)?;
    let area = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|([ax, ay, _], [bx, by, _])| (ax - ox) * (by - oy) - (ay - oy) * (bx - ox))
        .sum::<f64>();
    let mut normalized = points.to_vec();
    if area < 0.0
        && let Some((_, tail)) = normalized.split_first_mut()
    {
        tail.reverse();
    }
    Ok(normalized)
}

/// Resolve the cursor ray against the three construction axes through a world reference.
/// A screen gesture can select Z as well as X/Y without flattening onto a ground plane.
pub fn orthogonal_cursor(
    reference: [f64; 3],
    origin: [f64; 3],
    direction: [f64; 3],
    frame: Option<crate::work_plane::WorkPlane>,
) -> Result<[f64; 3], CommandError> {
    let frame = frame.unwrap_or_default();
    frame.to_local(reference)?;
    frame.to_local(origin)?;
    if !direction.iter().all(|n| n.is_finite()) {
        return Err(non_finite());
    }
    let length = direction.iter().map(|v| v * v).sum::<f64>().sqrt();
    if !length.is_finite() || length < 1e-9 {
        return Err(invalid());
    }
    let d = direction.map(|v| v / length);
    let dot = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
    let delta = std::array::from_fn(|i| {
        reference.get(i).copied().unwrap_or(0.0) - origin.get(i).copied().unwrap_or(0.0)
    });
    let mut best: Option<(f64, [f64; 3])> = None;
    for axis in [frame.x(), frame.y(), frame.normal()] {
        let da = dot(d, axis);
        let denominator = 1.0 - da * da;
        if denominator < 1e-10 {
            continue;
        }
        let t = (dot(delta, d) - dot(delta, axis) * da) / denominator;
        if t <= 0.0 {
            continue;
        }
        let s = t * da - dot(delta, axis);
        let point = std::array::from_fn(|i| {
            reference.get(i).copied().unwrap_or(0.0) + axis.get(i).copied().unwrap_or(0.0) * s
        });
        if frame.to_local(point).is_err() {
            continue;
        }
        let residual = std::array::from_fn(|i| {
            delta.get(i).copied().unwrap_or(0.0) + axis.get(i).copied().unwrap_or(0.0) * s
                - d.get(i).copied().unwrap_or(0.0) * t
        });
        let score = dot(residual, residual) / (t * t);
        if best.is_none_or(|(b, _)| score < b) {
            best = Some((score, point));
        }
    }
    best.map(|(_, p)| p).ok_or_else(invalid)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn absolute_relative_and_polar_share_the_reference_and_lock() {
        let origin = [100.0, 200.0, 300.0];
        assert_eq!(
            resolve("1,2,3", origin, AxisLock::None).unwrap(),
            [1., 2., 3.]
        );
        assert_eq!(
            resolve("@3,4,5", origin, AxisLock::None).unwrap(),
            [103., 204., 305.]
        );
        assert_eq!(
            resolve("@500<0,20", origin, AxisLock::None).unwrap(),
            [600., 200., 320.]
        );
        let p = resolve("@500<1.5707963267948966", origin, AxisLock::None).unwrap();
        assert!((p[0] - 100.0).abs() < 1e-10);
        assert!((p[1] - 700.0).abs() < 1e-10);
        assert_eq!(p[2], 300.0);
        for (lock, expected) in [
            (AxisLock::X, [103., 200., 300.]),
            (AxisLock::Y, [100., 204., 300.]),
            (AxisLock::Z, [100., 200., 305.]),
        ] {
            assert_eq!(resolve("@3,4,5", origin, lock).unwrap(), expected);
        }
    }

    #[test]
    fn invalid_or_overflowed_input_is_refused_even_on_a_locked_axis() {
        for text in [
            "",
            "1,2",
            "1,2,3,4",
            "NaN,0,0",
            "@-1<0",
            "10<0",
            "@1<inf",
            "@1<0,1,2",
            "@1e309,0,0",
        ] {
            assert!(resolve(text, [0.; 3], AxisLock::X).is_err(), "{text}");
        }
        assert!(resolve("@1e308,0,0", [1e308, 0., 0.], AxisLock::Y).is_err());
        assert!(resolve(&"0".repeat(129), [0.; 3], AxisLock::None).is_err());
    }

    #[test]
    fn length_and_ortho_resolve_in_three_dimensions_and_refuse_invalid_steps() {
        let p =
            construction_step([10., 20., 30.], [13., 24., 42.], Some(500.), false, None).unwrap();
        let length = p
            .iter()
            .zip([10., 20., 30.])
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
            .sqrt();
        assert!((length - 500.).abs() < 1e-10);
        assert_eq!(
            construction_step([10., 20., 30.], [13., 24., 42.], Some(500.), true, None).unwrap(),
            [10., 20., 530.]
        );
        for distance in [0., -1., f64::NAN, f64::INFINITY, 1e30] {
            assert!(construction_step([0.; 3], [1.; 3], Some(distance), false, None).is_err());
        }
        assert!(construction_step([0.; 3], [0.; 3], Some(500.), false, None).is_err());
        let p = orthogonal_cursor(
            [0.; 3],
            [1000., -1000., 1000.],
            [-1000., 1000., -400.],
            None,
        )
        .unwrap();
        assert!(
            p.iter()
                .zip([0., 0., 600.])
                .all(|(a, b)| (a - b).abs() < 1e-8)
        );
        let plane = contour_frame(
            &[
                [0., 0., 0.],
                [500., 0., 0.],
                [500., 0., 300.],
                [0., 0., 300.],
            ],
            None,
        )
        .unwrap();
        assert!(plane.to_local([0., 0., 300.]).unwrap()[2].abs() < 1e-8);
        assert!(
            contour_frame(
                &[
                    [0., 0., 0.],
                    [500., 0., 0.],
                    [500., 0., 300.],
                    [0., 1., 300.]
                ],
                None
            )
            .is_err()
        );
    }

    #[test]
    fn inferred_spatial_contour_normal_does_not_depend_on_traversal() {
        let points = [
            [0., 0., 0.],
            [500., 0., 0.],
            [500., 0., 300.],
            [0., 0., 300.],
        ];
        let mut reversed = points;
        reversed[1..].reverse();
        let a = contour_frame(&points, None).unwrap();
        let b = contour_frame(&reversed, None).unwrap();
        assert_eq!(a.normal(), b.normal());
        assert_eq!(a.normal(), [0., 1., 0.]);
        let preferred =
            crate::work_plane::WorkPlane::new([0.; 3], [1., 0., 0.], [0., 0., 1.]).unwrap();
        assert_eq!(
            contour_frame(&reversed, Some(preferred)).unwrap(),
            preferred
        );
    }
}
