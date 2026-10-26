// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T01.3 requests for regular-family and grid-frame updates.
use crate::{
    command::{CommandError, label_is_valid},
    digest::{ContentDigest, Sha256},
    error::ErrorCode,
    grid::{
        GridExpressionError, MAX_GRID_LINES_PER_FAMILY, parse_grid_expression, validate_grid_labels,
    },
    id::ElementId,
};

/// A regular-family replacement or span-only change. Expression and labels are supplied together.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RegularGridPatch {
    /// New authoritative source expression.
    pub expression: Option<String>,
    /// Full label list in generated position order.
    pub labels: Option<Vec<String>>,
    /// Fresh AxisIds for labels absent from the old family, in new-line order.
    pub new_axis_ids: Vec<ElementId>,
    /// Explicit replacement span; omission retains the stored span.
    pub span_mm: Option<[f64; 2]>,
}
/// One atomic grid update. Omitted fields retain their current values.
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateGrid {
    /// Existing GridId.
    pub target_id: ElementId,
    /// Optional grid rename; names need not be unique.
    pub name: Option<String>,
    /// Optional world XY origin in millimetres.
    pub origin_mm: Option<[f64; 2]>,
    /// Optional Z/W components of a Z-only rigid orientation.
    pub orientation_zw: Option<[f64; 2]>,
    /// Optional regular U-family update.
    pub u: Option<RegularGridPatch>,
    /// Optional regular V-family update.
    pub v: Option<RegularGridPatch>,
}
fn bad(code: ErrorCode, msg: &'static str) -> CommandError {
    CommandError::new(code, "grid.update", msg).with_rule("T01-V01")
}
impl UpdateGrid {
    /// Admit bounded arguments before looking up the target grid.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        if self.name.is_none()
            && self.origin_mm.is_none()
            && self.orientation_zw.is_none()
            && self.u.is_none()
            && self.v.is_none()
        {
            return Err(bad(ErrorCode::InvalidArgument, "empty grid update"));
        }
        if self.name.as_ref().is_some_and(|s| !label_is_valid(s)) {
            return Err(bad(ErrorCode::InvalidArgument, "invalid grid name"));
        }
        if let Some(p) = self.origin_mm {
            if p.iter().any(|n| !n.is_finite()) {
                return Err(bad(ErrorCode::NonFiniteNumber, "grid origin is not finite"));
            }
            if p.iter().any(|n| n.abs() > 1e9) {
                return Err(bad(
                    ErrorCode::LimitExceeded,
                    "grid origin exceeds the coordinate cap",
                ));
            }
        }
        if let Some([z, w]) = self.orientation_zw {
            if !z.is_finite() || !w.is_finite() {
                return Err(bad(
                    ErrorCode::NonFiniteNumber,
                    "grid orientation is not finite",
                ));
            }
            if z.abs() > 1.0 || w.abs() > 1.0 || ((z * z + w * w) - 1.0).abs() > 1e-6 {
                return Err(bad(ErrorCode::InvalidArgument, "invalid grid orientation"));
            }
        }
        for p in [&self.u, &self.v].into_iter().flatten() {
            if p.new_axis_ids.len() > MAX_GRID_LINES_PER_FAMILY
                || p.labels
                    .as_ref()
                    .is_some_and(|l| l.len() > MAX_GRID_LINES_PER_FAMILY)
            {
                return Err(bad(
                    ErrorCode::LimitExceeded,
                    "grid family exceeds the line cap",
                ));
            }
            if p.expression.is_some() != p.labels.is_some() {
                return Err(bad(ErrorCode::InvalidArgument, "GRID_LABEL_COUNT"));
            }
            if p.expression.is_none() && !p.new_axis_ids.is_empty() {
                return Err(bad(
                    ErrorCode::InvalidArgument,
                    "axis identities require a family expression",
                ));
            }
            if p.expression.is_none() && p.span_mm.is_none() {
                return Err(bad(ErrorCode::InvalidArgument, "empty family update"));
            }
            if let (Some(expression), Some(labels)) = (&p.expression, &p.labels) {
                let (_, positions) = parse_grid_expression(expression).map_err(|e| {
                    bad(
                        match e {
                            GridExpressionError::Limit => ErrorCode::LimitExceeded,
                            GridExpressionError::NonFinite => ErrorCode::NonFiniteNumber,
                            _ => ErrorCode::InvalidArgument,
                        },
                        match e {
                            GridExpressionError::Spacing => "GRID_SPACING_INVALID",
                            _ => "GRID_EXPRESSION_INVALID",
                        },
                    )
                })?;
                if positions.len() != labels.len() {
                    return Err(bad(ErrorCode::InvalidArgument, "GRID_LABEL_COUNT"));
                }
                validate_grid_labels(labels)
                    .map_err(|_| bad(ErrorCode::InvalidArgument, "GRID_LABEL_INVALID"))?;
            }
            if let Some([a, b]) = p.span_mm {
                if !a.is_finite() || !b.is_finite() {
                    return Err(bad(ErrorCode::NonFiniteNumber, "grid span is not finite"));
                }
                if a.abs() > 1e6 || b.abs() > 1e6 {
                    return Err(bad(
                        ErrorCode::LimitExceeded,
                        "grid span exceeds the coordinate cap",
                    ));
                }
                if b - a < 1.0 {
                    return Err(bad(ErrorCode::InvalidArgument, "GRID_LINE_DEGENERATE"));
                }
            }
        }
        Ok(())
    }
    /// Deterministic commitment to the request, independent of the current grid state.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let mut out = b"BeamCraft/T01/UpdateGrid/v1\0".to_vec();
        out.extend_from_slice(self.target_id.as_bytes());
        let string = |out: &mut Vec<u8>, s: &str| {
            out.extend_from_slice(&(s.len() as u64).to_le_bytes());
            out.extend_from_slice(s.as_bytes());
        };
        let pair = |out: &mut Vec<u8>, p: Option<[f64; 2]>| -> Result<(), CommandError> {
            match p {
                None => out.push(0),
                Some(p) => {
                    out.push(1);
                    for n in p {
                        out.extend_from_slice(&beamcraft_geom::canonical_f64_bytes(n).map_err(
                            |_| bad(ErrorCode::NonFiniteNumber, "non-finite grid update"),
                        )?);
                    }
                }
            }
            Ok(())
        };
        match &self.name {
            None => out.push(0),
            Some(s) => {
                out.push(1);
                string(&mut out, s);
            }
        }
        pair(&mut out, self.origin_mm)?;
        pair(&mut out, self.orientation_zw)?;
        for patch in [&self.u, &self.v] {
            match patch {
                None => out.push(0),
                Some(p) => {
                    out.push(1);
                    match &p.expression {
                        None => out.push(0),
                        Some(s) => {
                            out.push(1);
                            string(&mut out, s);
                        }
                    }
                    match &p.labels {
                        None => out.push(0),
                        Some(labels) => {
                            out.push(1);
                            out.extend_from_slice(&(labels.len() as u64).to_le_bytes());
                            for s in labels {
                                string(&mut out, s);
                            }
                        }
                    }
                    out.extend_from_slice(&(p.new_axis_ids.len() as u64).to_le_bytes());
                    for id in &p.new_axis_ids {
                        out.extend_from_slice(id.as_bytes());
                    }
                    pair(&mut out, p.span_mm)?;
                }
            }
        }
        Sha256::digest(&out).map_err(|_| {
            bad(
                ErrorCode::LimitExceeded,
                "grid update exceeds the encoding cap",
            )
        })
    }
}
