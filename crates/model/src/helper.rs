// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T02 construction geometry. These are snap aids, never engineering parts.

use crate::{
    command::{CommandError, label_is_valid},
    digest::{ContentDigest, Sha256},
    error::ErrorCode,
    id::ElementId,
};

/// Most helper elements in one project.
pub const MAX_HELPERS: usize = 1024;

/// Bounded world-space construction geometry, in millimetres.
#[derive(Debug, Clone, PartialEq)]
pub enum HelperGeometry {
    /// One snap point.
    Point([f64; 3]),
    /// A finite segment, including its endpoints and midpoint.
    Line([f64; 3], [f64; 3]),
    /// A circle given by center, nonzero normal and radius.
    Circle {
        /// Center in world coordinates.
        center: [f64; 3],
        /// Plane normal; need not be unit length.
        normal: [f64; 3],
        /// Positive radius in millimetres.
        radius_mm: f64,
    },
    /// An infinite construction plane.
    Plane {
        /// A point on the plane.
        origin: [f64; 3],
        /// Nonzero normal; need not be unit length.
        normal: [f64; 3],
    },
}

fn invalid() -> CommandError {
    CommandError::new(
        ErrorCode::InvalidArgument,
        "helper",
        "invalid construction geometry",
    )
}

impl HelperGeometry {
    /// The exact stored kind token.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Point(_) => "point",
            Self::Line(..) => "line",
            Self::Circle { .. } => "circle",
            Self::Plane { .. } => "plane",
        }
    }
    /// Explicit ordered numeric inputs: point, endpoints, center/normal/radius, or origin/normal.
    pub fn parameters(&self) -> Vec<f64> {
        match self {
            Self::Point(p) => p.to_vec(),
            Self::Line(a, b) => a.iter().chain(b).copied().collect(),
            Self::Circle {
                center,
                normal,
                radius_mm,
            } => center
                .iter()
                .chain(normal)
                .copied()
                .chain([*radius_mm])
                .collect(),
            Self::Plane { origin, normal } => origin.iter().chain(normal).copied().collect(),
        }
    }
    /// Decodes a bounded explicit record and validates it before use.
    pub fn from_parameters(kind: &str, values: &[f64]) -> Result<Self, CommandError> {
        let geometry = match (kind, values) {
            ("point", &[x, y, z]) => Self::Point([x, y, z]),
            ("line", &[ax, ay, az, bx, by, bz]) => Self::Line([ax, ay, az], [bx, by, bz]),
            ("circle", &[x, y, z, nx, ny, nz, radius_mm]) => Self::Circle {
                center: [x, y, z],
                normal: [nx, ny, nz],
                radius_mm,
            },
            ("plane", &[x, y, z, nx, ny, nz]) => Self::Plane {
                origin: [x, y, z],
                normal: [nx, ny, nz],
            },
            _ => return Err(invalid()),
        };
        geometry.validate()?;
        Ok(geometry)
    }
    /// Refuses non-finite, out-of-range and degenerate geometry.
    pub fn validate(&self) -> Result<(), CommandError> {
        let values = self.parameters();
        if values.iter().any(|v| !v.is_finite()) {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "helper",
                "construction inputs must be finite",
            ));
        }
        if values.iter().any(|v| v.abs() > 1e9) {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "helper",
                "construction coordinate cap exceeded",
            ));
        }
        match self {
            Self::Point(_) => {}
            Self::Line(a, b) => {
                let length = a
                    .iter()
                    .zip(b)
                    .map(|(a, b)| (b - a).powi(2))
                    .sum::<f64>()
                    .sqrt();
                if !(0.001..=1e6).contains(&length) {
                    return Err(invalid());
                }
            }
            Self::Circle {
                normal,
                radius_mm,
                center,
            } => {
                check_normal(*normal)?;
                if !(0.001..=1e6).contains(radius_mm)
                    || center.iter().any(|v| v.abs() + radius_mm > 1e9)
                {
                    return Err(invalid());
                }
            }
            Self::Plane { normal, .. } => check_normal(*normal)?,
        }
        Ok(())
    }
}
fn check_normal(n: [f64; 3]) -> Result<(), CommandError> {
    let length = n.iter().map(|v| v * v).sum::<f64>().sqrt();
    if length < 1e-12 {
        Err(invalid())
    } else {
        Ok(())
    }
}

/// Authored helper element; its geometry is independent of steel parts.
#[derive(Debug, Clone, PartialEq)]
pub struct Helper {
    /// Stable identity.
    pub id: ElementId,
    /// User label.
    pub name: String,
    /// Validated construction geometry.
    pub geometry: HelperGeometry,
}
impl Helper {
    /// Validates all authored fields.
    pub fn validate(&self) -> Result<(), CommandError> {
        if !label_is_valid(&self.name) {
            return Err(invalid());
        }
        self.geometry.validate()
    }
    /// Fingerprint of the complete creation request; signed zero is canonicalized.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate()?;
        let mut bytes = b"BeamCraft/T02/CreateHelper/v1\0".to_vec();
        bytes.extend_from_slice(self.id.as_bytes());
        bytes.extend_from_slice(&(self.name.len() as u64).to_le_bytes());
        bytes.extend_from_slice(self.name.as_bytes());
        bytes.extend_from_slice(self.geometry.kind().as_bytes());
        for v in self.geometry.parameters() {
            bytes.extend_from_slice(&(if v == 0.0 { 0.0 } else { v }).to_le_bytes());
        }
        Sha256::digest(&bytes).map_err(|_| invalid())
    }
}
