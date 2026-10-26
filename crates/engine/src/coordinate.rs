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
}
