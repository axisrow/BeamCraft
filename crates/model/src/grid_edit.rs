// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T01.2 typed edits of drawn grid lines and explicit labels.
use crate::{
    command::CommandError,
    digest::{ContentDigest, Sha256},
    error::ErrorCode,
    grid::{GridLabelError, MAX_GRID_LINES_PER_FAMILY, validate_grid_labels},
    id::ElementId,
};

/// One of a grid's two ordered families.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridFamilyId {
    /// U lines.
    U,
    /// V lines.
    V,
}
impl GridFamilyId {
    /// Parse the exact public family token.
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "u" => Some(Self::U),
            "v" => Some(Self::V),
            _ => None,
        }
    }
    /// The exact family token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::U => "u",
            Self::V => "v",
        }
    }
}
/// One atomic T01.2 operation. Coordinates are world XY millimetres; grids have no elevation.
#[derive(Debug, Clone, PartialEq)]
pub enum GridLineEdit {
    /// Add one line to a drawn family, using a caller-supplied fresh identity.
    Draw {
        /// Destination family.
        family: GridFamilyId,
        /// Fresh AxisId.
        new_axis_id: ElementId,
        /// Explicit label.
        label: String,
        /// First world XY point.
        from_mm: [f64; 2],
        /// Second world XY point.
        to_mm: [f64; 2],
    },
    /// Reshape one drawn line without changing its name or identity.
    Reshape {
        /// Existing AxisId.
        axis_id: ElementId,
        /// First world XY point.
        from_mm: [f64; 2],
        /// Second world XY point.
        to_mm: [f64; 2],
    },
    /// Remove one drawn line, subject to the last-line and reference guards.
    Remove {
        /// Existing AxisId.
        axis_id: ElementId,
    },
    /// Change one label in either family without changing geometry or identity.
    Relabel {
        /// Existing AxisId.
        axis_id: ElementId,
        /// New explicit label.
        label: String,
    },
    /// Replace all labels of one family in its existing order.
    RelabelFamily {
        /// Destination family.
        family: GridFamilyId,
        /// Complete explicit label list.
        labels: Vec<String>,
    },
}
/// An atomic line edit of one grid.
#[derive(Debug, Clone, PartialEq)]
pub struct GridLineEditRequest {
    /// Existing GridId.
    pub target_id: ElementId,
    /// The requested line operation.
    pub edit: GridLineEdit,
}
fn bad(code: ErrorCode, message: &'static str) -> CommandError {
    CommandError::new(code, "grid.line", message).with_rule("T01-V01")
}
impl GridLineEditRequest {
    /// Admit bounded labels and finite, bounded world points before consulting project state.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        let labels = match &self.edit {
            GridLineEdit::Draw { label, .. } | GridLineEdit::Relabel { label, .. } => {
                std::slice::from_ref(label)
            }
            GridLineEdit::RelabelFamily { labels, .. } => labels.as_slice(),
            _ => &[],
        };
        if labels.len() > MAX_GRID_LINES_PER_FAMILY {
            return Err(bad(ErrorCode::LimitExceeded, "too many grid labels"));
        }
        validate_grid_labels(labels).map_err(|e| {
            bad(
                ErrorCode::InvalidArgument,
                match e {
                    GridLabelError::Duplicate => "GRID_LABEL_DUPLICATE",
                    _ => "GRID_LABEL_INVALID",
                },
            )
        })?;
        let endpoints = match &self.edit {
            GridLineEdit::Draw { from_mm, to_mm, .. }
            | GridLineEdit::Reshape { from_mm, to_mm, .. } => Some((*from_mm, *to_mm)),
            _ => None,
        };
        if let Some((a, b)) = endpoints {
            if a.into_iter().chain(b).any(|n| !n.is_finite()) {
                return Err(bad(
                    ErrorCode::NonFiniteNumber,
                    "grid endpoint is not finite",
                ));
            }
            if a.into_iter().chain(b).any(|n| n.abs() > 1e9) {
                return Err(bad(
                    ErrorCode::LimitExceeded,
                    "grid world endpoint exceeds the coordinate cap",
                ));
            }
            let [ax, ay] = a;
            let [bx, by] = b;
            if (bx - ax).hypot(by - ay) < 1.0 {
                return Err(bad(ErrorCode::InvalidArgument, "GRID_LINE_DEGENERATE"));
            }
        }
        Ok(())
    }
    /// Deterministic request commitment, independent of the current grid, for exact replay.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let mut out = b"BeamCraft/T01/GridLineEdit/v1\0".to_vec();
        out.extend_from_slice(self.target_id.as_bytes());
        let label = |out: &mut Vec<u8>, s: &str| {
            out.extend_from_slice(&(s.len() as u64).to_le_bytes());
            out.extend_from_slice(s.as_bytes());
        };
        let point =
            |out: &mut Vec<u8>, p: [f64; 2]| -> Result<(), CommandError> {
                for n in p {
                    out.extend_from_slice(&beamcraft_geom::canonical_f64_bytes(n).map_err(
                        |_| bad(ErrorCode::NonFiniteNumber, "grid endpoint is not finite"),
                    )?);
                }
                Ok(())
            };
        match &self.edit {
            GridLineEdit::Draw {
                family,
                new_axis_id,
                label: name,
                from_mm,
                to_mm,
            } => {
                out.push(0);
                out.extend_from_slice(family.as_str().as_bytes());
                out.extend_from_slice(new_axis_id.as_bytes());
                label(&mut out, name);
                point(&mut out, *from_mm)?;
                point(&mut out, *to_mm)?;
            }
            GridLineEdit::Reshape {
                axis_id,
                from_mm,
                to_mm,
            } => {
                out.push(1);
                out.extend_from_slice(axis_id.as_bytes());
                point(&mut out, *from_mm)?;
                point(&mut out, *to_mm)?;
            }
            GridLineEdit::Remove { axis_id } => {
                out.push(2);
                out.extend_from_slice(axis_id.as_bytes());
            }
            GridLineEdit::Relabel {
                axis_id,
                label: name,
            } => {
                out.push(3);
                out.extend_from_slice(axis_id.as_bytes());
                label(&mut out, name);
            }
            GridLineEdit::RelabelFamily { family, labels } => {
                out.push(4);
                out.extend_from_slice(family.as_str().as_bytes());
                out.extend_from_slice(&(labels.len() as u64).to_le_bytes());
                for name in labels {
                    label(&mut out, name);
                }
            }
        }
        Sha256::digest(&out).map_err(|_| {
            bad(
                ErrorCode::LimitExceeded,
                "grid edit request exceeds the encoding cap",
            )
        })
    }
}
