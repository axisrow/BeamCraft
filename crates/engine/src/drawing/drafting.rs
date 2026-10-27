// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D07 slice 1: drawn geometry primitives owned by a drawing document.
//!
//! A drafting entity lives on the drawing's paper in millimetres and never references, probes or
//! mutates the engineering model. Identity follows the D01 rules: one full canonical UUIDv7 per
//! entity, pairwise distinct inside the document. Angles are radians, right-handed, counter-
//! clockwise from paper X.

use super::{MAX_PAPER_MM, invalid};
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::DraftingEntityId;

/// Largest number of drawn objects in one document. With the three fixed document prims and the
/// drafting scope this keeps every intent page within the layer engine's 64-item page ceiling.
pub const MAX_DRAFTING_ENTITIES: usize = 60;
/// Largest number of vertices of one drawn polyline.
pub const MAX_POLYLINE_POINTS: usize = 16;

/// One complete turn in radians; the bound for an admitted arc angle magnitude.
pub const MAX_ARC_TURN: f64 = std::f64::consts::TAU;

/// One drawn paper-space object. Geometry is exact authored intent; there is no snapping,
/// inference or model lookup behind it.
#[derive(Debug, Clone, PartialEq)]
pub enum DraftingGeometry {
    /// A straight segment between two distinct points.
    Line {
        /// First endpoint.
        a: [f64; 2],
        /// Second endpoint, distinct from `a`.
        b: [f64; 2],
    },
    /// A connected run of two to sixteen points with no zero-length segment.
    Polyline {
        /// Vertices in drawn order.
        points: Vec<[f64; 2]>,
    },
    /// A circular arc about a centre, from the start angle to the end angle counter-clockwise.
    Arc {
        /// Centre in paper millimetres.
        center: [f64; 2],
        /// Radius in paper millimetres, greater than zero.
        radius: f64,
        /// Start angle in radians, magnitude at most one turn.
        start: f64,
        /// End angle in radians, magnitude at most one turn; the sweep must be nonzero.
        end: f64,
    },
    /// A full circle about a centre.
    Circle {
        /// Centre in paper millimetres.
        center: [f64; 2],
        /// Radius in paper millimetres, greater than zero.
        radius: f64,
    },
    /// An axis-aligned rectangle between two opposite corners with positive width and height.
    Rectangle {
        /// First corner.
        a: [f64; 2],
        /// Opposite corner, neither edge of the rectangle may be degenerate.
        b: [f64; 2],
    },
}

impl DraftingGeometry {
    /// Stable lowercase wire token, also the persisted `drawing:kind` value.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Line { .. } => "line",
            Self::Polyline { .. } => "polyline",
            Self::Arc { .. } => "arc",
            Self::Circle { .. } => "circle",
            Self::Rectangle { .. } => "rectangle",
        }
    }

    /// The flat paper-coordinate payload in the canonical persisted order.
    pub fn coordinates(self) -> Vec<f64> {
        fn push(out: &mut Vec<f64>, [x, y]: [f64; 2]) {
            out.push(x);
            out.push(y);
        }
        let mut out = Vec::new();
        match self {
            Self::Line { a, b } => {
                push(&mut out, a);
                push(&mut out, b);
            }
            Self::Polyline { points } => {
                for point in points {
                    push(&mut out, point);
                }
            }
            Self::Arc {
                center,
                radius,
                start,
                end,
            } => {
                push(&mut out, center);
                out.push(radius);
                out.push(start);
                out.push(end);
            }
            Self::Circle { center, radius } => {
                push(&mut out, center);
                out.push(radius);
            }
            Self::Rectangle { a, b } => {
                push(&mut out, a);
                push(&mut out, b);
            }
        }
        out
    }

    /// Rebuilds geometry from a kind token and its flat coordinate payload. Used by the closed
    /// intent reader; validation still runs in [`validate_geometry`].
    pub fn from_parts(kind: &str, values: &[f64]) -> Option<Self> {
        fn point(values: &[f64], index: usize) -> Option<[f64; 2]> {
            fn pair(pair: &[f64]) -> Option<[f64; 2]> {
                match pair {
                    [x, y] => Some([*x, *y]),
                    _ => None,
                }
            }
            pair(values.get(index..index.checked_add(2)?)?)
        }
        let number = |index: usize| values.get(index).copied();
        match kind {
            "line" => Some(Self::Line {
                a: point(values, 0)?,
                b: point(values, 2)?,
            }),
            "rectangle" => Some(Self::Rectangle {
                a: point(values, 0)?,
                b: point(values, 2)?,
            }),
            "circle" => Some(Self::Circle {
                center: point(values, 0)?,
                radius: number(2)?,
            }),
            "arc" => Some(Self::Arc {
                center: point(values, 0)?,
                radius: number(2)?,
                start: number(3)?,
                end: number(4)?,
            }),
            "polyline" if values.len().is_multiple_of(2) => {
                let points = values
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|&pair| Some(pair))
                    .collect::<Option<Vec<_>>>()?;
                Some(Self::Polyline { points })
            }
            _ => None,
        }
    }
}

/// One drawn object: identity plus paper geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct DraftingSpec {
    /// Stable entity identity, distinct from every other document object UUID.
    pub id: DraftingEntityId,
    /// The drawn geometry.
    pub geometry: DraftingGeometry,
}

fn finite_point(point: [f64; 2]) -> Result<(), CommandError> {
    for value in point {
        if !value.is_finite() {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "drafting",
                "coordinate must be finite",
            ));
        }
        if value.abs() > MAX_PAPER_MM {
            return Err(invalid(
                "drafting",
                "coordinate is outside the drawing contract's range",
            ));
        }
    }
    Ok(())
}

/// Validates one geometry against the D07 slice bounds. Public because the closed intent reader
/// rebuilds geometry from persisted bytes and must pass it through the same bounds.
pub fn validate_geometry(geometry: &DraftingGeometry) -> Result<(), CommandError> {
    match geometry {
        DraftingGeometry::Line { a, b } => {
            finite_point(*a)?;
            finite_point(*b)?;
            if a == b {
                return Err(invalid("drafting", "line endpoints must be distinct"));
            }
        }
        DraftingGeometry::Polyline { points } => {
            if points.len() < 2 || points.len() > MAX_POLYLINE_POINTS {
                return Err(invalid(
                    "drafting",
                    "polyline needs two through sixteen points",
                ));
            }
            let mut previous: Option<[f64; 2]> = None;
            for point in points {
                finite_point(*point)?;
                if previous == Some(*point) {
                    return Err(invalid(
                        "drafting",
                        "polyline segments must have positive length",
                    ));
                }
                previous = Some(*point);
            }
        }
        DraftingGeometry::Arc {
            center,
            radius,
            start,
            end,
        } => {
            finite_point(*center)?;
            if !radius.is_finite() || !start.is_finite() || !end.is_finite() {
                return Err(CommandError::new(
                    ErrorCode::NonFiniteNumber,
                    "drafting",
                    "number must be finite",
                ));
            }
            if *radius <= 0.0 || *radius > MAX_PAPER_MM {
                return Err(invalid(
                    "drafting",
                    "radius must be positive and within the drawing contract's range",
                ));
            }
            if start.abs() > MAX_ARC_TURN || end.abs() > MAX_ARC_TURN {
                return Err(invalid(
                    "drafting",
                    "arc angles must stay within one turn in magnitude",
                ));
            }
            // A zero or full-turn sweep is a circle, never a silently accepted arc.
            if (end - start).rem_euclid(std::f64::consts::TAU) == 0.0 {
                return Err(invalid(
                    "drafting",
                    "arc sweep must be nonzero; use a circle for a full turn",
                ));
            }
        }
        DraftingGeometry::Circle { center, radius } => {
            finite_point(*center)?;
            if !radius.is_finite() {
                return Err(CommandError::new(
                    ErrorCode::NonFiniteNumber,
                    "drafting",
                    "number must be finite",
                ));
            }
            if *radius <= 0.0 || *radius > MAX_PAPER_MM {
                return Err(invalid(
                    "drafting",
                    "radius must be positive and within the drawing contract's range",
                ));
            }
        }
        DraftingGeometry::Rectangle { a, b } => {
            finite_point(*a)?;
            finite_point(*b)?;
            if a[0] == b[0] || a[1] == b[1] {
                return Err(invalid(
                    "drafting",
                    "rectangle needs positive width and height",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn line() -> DraftingGeometry {
        DraftingGeometry::Line {
            a: [0.0, 0.0],
            b: [10.0, 0.0],
        }
    }

    #[test]
    fn bounds_and_degenerate_shapes_refuse() {
        assert!(validate_geometry(&line()).is_ok());
        assert!(
            validate_geometry(&DraftingGeometry::Line {
                a: [1.0, 1.0],
                b: [1.0, 1.0]
            })
            .is_err()
        );
        assert!(
            validate_geometry(&DraftingGeometry::Line {
                a: [MAX_PAPER_MM + 1.0, 0.0],
                b: [0.0, 0.0]
            })
            .is_err()
        );
        assert!(
            validate_geometry(&DraftingGeometry::Circle {
                center: [0.0, 0.0],
                radius: 0.0
            })
            .is_err()
        );
        assert!(
            validate_geometry(&DraftingGeometry::Rectangle {
                a: [0.0, 0.0],
                b: [5.0, 0.0]
            })
            .is_err()
        );
        assert!(
            validate_geometry(&DraftingGeometry::Polyline {
                points: vec![[0.0, 0.0], [0.0, 0.0], [4.0, 4.0]]
            })
            .is_err()
        );
        assert!(
            validate_geometry(&DraftingGeometry::Arc {
                center: [0.0, 0.0],
                radius: 5.0,
                start: 1.0,
                end: 1.0 - std::f64::consts::TAU,
            })
            .is_err()
        );
    }

    #[test]
    fn coordinate_round_trip_through_parts_is_exact() {
        for geometry in [
            line(),
            DraftingGeometry::Circle {
                center: [3.0, 4.0],
                radius: 2.5,
            },
            DraftingGeometry::Arc {
                center: [3.0, 4.0],
                radius: 2.5,
                start: 0.0,
                end: 1.5,
            },
            DraftingGeometry::Rectangle {
                a: [0.0, 0.0],
                b: [5.0, 7.0],
            },
            DraftingGeometry::Polyline {
                points: vec![[0.0, 0.0], [1.0, 2.0], [4.0, -1.0]],
            },
        ] {
            let kind = geometry.kind();
            let values = geometry.clone().coordinates();
            assert_eq!(
                DraftingGeometry::from_parts(kind, &values),
                Some(geometry),
                "kind {kind} must survive the flat wire form exactly"
            );
        }
    }
}
