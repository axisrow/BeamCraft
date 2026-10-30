// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T02 construction objects: helper points, lines, circles and planes.
//!
//! A construction object is an authored helper element with no engineering body: it is never
//! exported, scheduled or numbered, and exists so that snapping and placement have explicit
//! targets. Its geometry is authored directly in world millimetres; the element carries the
//! identity placement (like a weld or a level), so no pose is stored or admitted.

use crate::command::{CommandError, label_is_valid};
use crate::digest::{ContentDigest, Sha256};
use crate::error::ErrorCode;
use crate::id::ElementId;
use crate::limits::{COORDINATE_CAP_MM, DIMENSION_CAP_MM};
use beamcraft_geom::canonical_f64_bytes;

/// Most construction objects of one project.
pub const MAX_CONSTRUCTION_OBJECTS: usize = 4096;

/// The helper geometry of one construction object, in world millimetres.
///
/// A line is the closed segment between its two distinct end points. A circle is the
/// circumference about `centre_mm` in the plane through the centre perpendicular to `axis`;
/// `axis` may be submitted at any non-zero length and is used normalized. A plane is the set
/// `normal . (p - point_mm) = 0`; its normal is used normalized too.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConstructionGeometry {
    /// One helper point.
    Point {
        /// The point in world millimetres.
        point_mm: [f64; 3],
    },
    /// One helper line segment.
    Line {
        /// First end in world millimetres.
        start_mm: [f64; 3],
        /// Second end in world millimetres, distinct from `start_mm`.
        end_mm: [f64; 3],
    },
    /// One helper circle.
    Circle {
        /// The centre in world millimetres.
        centre_mm: [f64; 3],
        /// A non-zero normal of the circle's plane.
        axis: [f64; 3],
        /// The radius in millimetres, greater than zero.
        radius_mm: f64,
    },
    /// One helper plane.
    Plane {
        /// A point of the plane in world millimetres.
        point_mm: [f64; 3],
        /// A non-zero normal of the plane.
        normal: [f64; 3],
    },
}

impl ConstructionGeometry {
    /// The stored kind token: `point`, `line`, `circle` or `plane`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Point { .. } => "point",
            Self::Line { .. } => "line",
            Self::Circle { .. } => "circle",
            Self::Plane { .. } => "plane",
        }
    }

    /// Admits the geometry: every coordinate finite and within the coordinate cap
    /// (`NON_FINITE_NUMBER`, `LIMIT_EXCEEDED`), a line's ends distinct and a plane's or
    /// circle's direction non-zero (`INVALID_ARGUMENT`), a radius positive and within the
    /// dimension cap (`INVALID_ARGUMENT`, `LIMIT_EXCEEDED`).
    pub fn validate(&self) -> Result<(), CommandError> {
        let bad = |field: &'static str| {
            CommandError::new(ErrorCode::NonFiniteNumber, field, "value is not finite")
                .with_rule("DV-ATTR-003")
        };
        let cap = |field: &'static str| {
            CommandError::new(
                ErrorCode::LimitExceeded,
                field,
                "coordinate exceeds the coordinate cap",
            )
            .with_rule("DV-LIM-001")
        };
        let check = |field: &'static str, values: &[f64]| -> Result<(), CommandError> {
            if values.iter().any(|v| !v.is_finite()) {
                return Err(bad(field));
            }
            if values.iter().any(|v| v.abs() > COORDINATE_CAP_MM) {
                return Err(cap(field));
            }
            Ok(())
        };
        match self {
            Self::Point { point_mm } => check("point_mm", point_mm),
            Self::Line { start_mm, end_mm } => {
                check("start_mm", start_mm)?;
                check("end_mm", end_mm)?;
                if start_mm == end_mm {
                    return Err(CommandError::new(
                        ErrorCode::InvalidArgument,
                        "end_mm",
                        "the two line ends must differ",
                    ));
                }
                Ok(())
            }
            Self::Circle {
                centre_mm,
                axis,
                radius_mm,
            } => {
                check("centre_mm", centre_mm)?;
                check("axis", axis)?;
                if !radius_mm.is_finite() {
                    return Err(bad("radius_mm"));
                }
                if *radius_mm <= 0.0 {
                    return Err(CommandError::new(
                        ErrorCode::InvalidArgument,
                        "radius_mm",
                        "a circle radius must be positive",
                    ));
                }
                if *radius_mm > DIMENSION_CAP_MM {
                    return Err(CommandError::new(
                        ErrorCode::LimitExceeded,
                        "radius_mm",
                        "radius exceeds the dimension cap",
                    )
                    .with_rule("DV-LIM-001"));
                }
                if axis.iter().all(|v| *v == 0.0) {
                    return Err(CommandError::new(
                        ErrorCode::InvalidArgument,
                        "axis",
                        "the circle needs a non-zero axis",
                    ));
                }
                Ok(())
            }
            Self::Plane { point_mm, normal } => {
                check("point_mm", point_mm)?;
                check("normal", normal)?;
                if normal.iter().all(|v| *v == 0.0) {
                    return Err(CommandError::new(
                        ErrorCode::InvalidArgument,
                        "normal",
                        "the plane needs a non-zero normal",
                    ));
                }
                Ok(())
            }
        }
    }
}

/// A construction object: a named helper element with no engineering body (T02). It is a snap
/// and placement target only: schedules, numbering and exports never see it.
#[derive(Debug, Clone, PartialEq)]
pub struct ConstructionObject {
    /// Stable element identity.
    pub id: ElementId,
    /// Label, 1 to 256 UTF-8 bytes without control characters. Not an identity.
    pub name: String,
    /// The helper geometry in world millimetres.
    pub geometry: ConstructionGeometry,
}

impl ConstructionObject {
    /// Admits the label and the geometry, in that order.
    pub fn validate(&self) -> Result<(), CommandError> {
        if !label_is_valid(&self.name) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "name",
                "label must be 1 to 256 bytes without control characters",
            )
            .with_rule("DV-BASE-003"));
        }
        self.geometry.validate()
    }
}

/// T02.1 creation request.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateConstructionObject {
    /// The complete object to create.
    pub object: ConstructionObject,
}

impl CreateConstructionObject {
    /// Admits all model arguments before consulting project state.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        self.object.validate()
    }

    /// Versioned, length-prefixed fingerprint of all authored input, as [`crate::command`]
    /// commands do; it is not the F04 canonical form.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let bad = || {
            CommandError::new(
                ErrorCode::NonFiniteNumber,
                "construction",
                "value cannot be encoded",
            )
            .with_rule("DV-ATTR-003")
        };
        let mut out = b"BeamCraft/T02/CreateConstructionObject/v1\0".to_vec();
        out.extend_from_slice(self.object.id.as_bytes());
        let name = self.object.name.as_bytes();
        out.extend_from_slice(&(name.len() as u64).to_le_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(self.object.geometry.as_str().as_bytes());
        let mut scalars: Vec<f64> = Vec::with_capacity(7);
        match self.object.geometry {
            ConstructionGeometry::Point { point_mm } => scalars.extend(point_mm),
            ConstructionGeometry::Line { start_mm, end_mm } => {
                scalars.extend(start_mm);
                scalars.extend(end_mm);
            }
            ConstructionGeometry::Circle {
                centre_mm,
                axis,
                radius_mm,
            } => {
                scalars.extend(centre_mm);
                scalars.extend(axis);
                scalars.push(radius_mm);
            }
            ConstructionGeometry::Plane { point_mm, normal } => {
                scalars.extend(point_mm);
                scalars.extend(normal);
            }
        }
        for value in scalars {
            out.extend_from_slice(&canonical_f64_bytes(value).map_err(|_| bad())?);
        }
        Sha256::digest(&out).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    fn id(n: u32) -> ElementId {
        ElementId::parse(&format!("01990000-0000-7000-8000-{n:012x}")).unwrap()
    }

    fn point(z: f64) -> ConstructionObject {
        ConstructionObject {
            id: id(1),
            name: "P1".to_owned(),
            geometry: ConstructionGeometry::Point {
                point_mm: [1000.0, 2000.0, z],
            },
        }
    }

    #[test]
    fn valid_geometry_of_every_kind_is_admitted() {
        let objects = [
            point(0.0),
            ConstructionObject {
                id: id(2),
                name: "L1".to_owned(),
                geometry: ConstructionGeometry::Line {
                    start_mm: [0.0; 3],
                    end_mm: [3000.0, 0.0, 0.0],
                },
            },
            ConstructionObject {
                id: id(3),
                name: "C1".to_owned(),
                geometry: ConstructionGeometry::Circle {
                    centre_mm: [0.0; 3],
                    axis: [0.0, 0.0, 2.0],
                    radius_mm: 500.0,
                },
            },
            ConstructionObject {
                id: id(4),
                name: "F1".to_owned(),
                geometry: ConstructionGeometry::Plane {
                    point_mm: [0.0; 3],
                    normal: [0.0, 0.0, 1.0],
                },
            },
        ];
        for object in &objects {
            assert!(object.validate().is_ok(), "{}", object.name);
            assert!(
                CreateConstructionObject {
                    object: object.clone()
                }
                .content_fingerprint()
                .is_ok()
            );
        }
    }

    #[test]
    fn refusals_carry_the_stable_codes_and_fields() {
        let code = |object: ConstructionObject| object.validate().map(|_| ()).unwrap_err();
        let unnamed = ConstructionObject {
            name: String::new(),
            ..point(0.0)
        };
        assert_eq!(code(unnamed).field, "name");
        let non_finite = ConstructionObject {
            geometry: ConstructionGeometry::Point {
                point_mm: [0.0, f64::NAN, 0.0],
            },
            ..point(0.0)
        };
        assert_eq!(code(non_finite.clone()).code, ErrorCode::NonFiniteNumber);
        assert_eq!(code(non_finite).field, "point_mm");
        let over_cap = ConstructionObject {
            geometry: ConstructionGeometry::Point {
                point_mm: [0.0, 0.0, 1.1e9],
            },
            ..point(0.0)
        };
        assert_eq!(code(over_cap).code, ErrorCode::LimitExceeded);
        let degenerate = ConstructionObject {
            geometry: ConstructionGeometry::Line {
                start_mm: [0.0; 3],
                end_mm: [0.0; 3],
            },
            ..point(0.0)
        };
        assert_eq!(code(degenerate).field, "end_mm");
        let zero_axis = ConstructionObject {
            geometry: ConstructionGeometry::Circle {
                centre_mm: [0.0; 3],
                axis: [0.0; 3],
                radius_mm: 500.0,
            },
            ..point(0.0)
        };
        assert_eq!(code(zero_axis).field, "axis");
        let bad_radius = ConstructionObject {
            geometry: ConstructionGeometry::Circle {
                centre_mm: [0.0; 3],
                axis: [0.0, 0.0, 1.0],
                radius_mm: 0.0,
            },
            ..point(0.0)
        };
        assert_eq!(code(bad_radius).field, "radius_mm");
        let huge_radius = ConstructionObject {
            geometry: ConstructionGeometry::Circle {
                centre_mm: [0.0; 3],
                axis: [0.0, 0.0, 1.0],
                radius_mm: 1.1e6,
            },
            ..point(0.0)
        };
        assert_eq!(code(huge_radius).code, ErrorCode::LimitExceeded);
        let zero_normal = ConstructionObject {
            geometry: ConstructionGeometry::Plane {
                point_mm: [0.0; 3],
                normal: [0.0; 3],
            },
            ..point(0.0)
        };
        assert_eq!(code(zero_normal).field, "normal");
    }

    #[test]
    fn the_fingerprint_binds_the_authored_content() {
        let a = CreateConstructionObject { object: point(0.0) };
        let b = a.clone();
        assert_eq!(
            a.content_fingerprint().unwrap(),
            b.content_fingerprint().unwrap()
        );
        let moved = CreateConstructionObject {
            object: point(500.0),
        };
        assert_ne!(
            a.content_fingerprint().unwrap(),
            moved.content_fingerprint().unwrap()
        );
        let renamed = CreateConstructionObject {
            object: ConstructionObject {
                name: "P2".to_owned(),
                ..point(0.0)
            },
        };
        assert_ne!(
            a.content_fingerprint().unwrap(),
            renamed.content_fingerprint().unwrap()
        );
    }
}
