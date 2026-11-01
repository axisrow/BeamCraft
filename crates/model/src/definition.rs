// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Logical content of the immutable profile and material definitions (F04 section 4, F06
//! section 2, P02).
//!
//! A definition is pinned by the full content digest of its stored USD layer, which the layer
//! engine computes; this module only admits the logical values with the F04 section 3 codes and
//! caps. There is no catalogue, no commercial designation and no inferred default: a material
//! without a density keeps an unknown density and never selects a guessed one.

use crate::command::{CommandError, label_is_valid};
use crate::error::ErrorCode;
use crate::limits::MAX_DENSITY_KG_M3;

/// A physical (engineering) material. Display appearance is a separate reference and signature
/// (F04 section 4) and is not part of this value.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MaterialSpec {
    /// Density in kg/m3. `None` is unknown, never a default.
    pub density_kg_m3: Option<f64>,
    /// Declared steel grade, for example the grade named in the cited standard. A label only:
    /// it never selects a density or any other number.
    pub grade: Option<String>,
    /// Declared standard of the grade, a label only.
    pub standard: Option<String>,
    /// Readable name shown in listings and used to select the definition by name (A07.2). Part
    /// of the definition content, so another name is another digest. A label only.
    pub display_name: Option<String>,
}

/// Shared with the steel crate.
pub fn declared_label(value: &Option<String>, field: &'static str) -> Result<(), CommandError> {
    match value {
        Some(text) if !label_is_valid(text) => Err(CommandError::new(
            ErrorCode::InvalidArgument,
            field,
            "label is empty, too long or has control characters",
        )),
        _ => Ok(()),
    }
}

impl MaterialSpec {
    /// Admits the density: finite (`NON_FINITE_NUMBER`), at most 1,000,000 kg/m3
    /// (`LIMIT_EXCEEDED`) and not negative (`INVALID_ARGUMENT`; zero is the massless auxiliary material); and the labels.
    pub fn validate(&self) -> Result<(), CommandError> {
        declared_label(&self.grade, "grade")?;
        declared_label(&self.standard, "standard")?;
        declared_label(&self.display_name, "display_name")?;
        let Some(density) = self.density_kg_m3 else {
            return Ok(());
        };
        if !density.is_finite() {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "density_kg_m3",
                "value is not finite",
            )
            .with_rule("DV-ATTR-003"));
        }
        if density.abs() > MAX_DENSITY_KG_M3 {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "density_kg_m3",
                "density exceeds the density cap",
            ));
        }
        if density < 0.0 {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "density_kg_m3",
                "density must not be negative",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    fn density(value: Option<f64>) -> Result<(), ErrorCode> {
        MaterialSpec {
            density_kg_m3: value,
            ..MaterialSpec::default()
        }
        .validate()
        .map_err(|e| e.code)
    }

    #[test]
    fn material_labels_are_admitted_and_never_choose_a_density() {
        let material = MaterialSpec {
            grade: Some("S355".into()),
            standard: Some("EN 10025-2".into()),
            ..MaterialSpec::default()
        };
        assert_eq!(material.validate().map_err(|e| e.code), Ok(()));
        assert_eq!(material.density_kg_m3, None);
        let bad = MaterialSpec {
            grade: Some("x".repeat(257)),
            ..MaterialSpec::default()
        };
        assert_eq!(
            bad.validate().map_err(|e| e.code),
            Err(ErrorCode::InvalidArgument)
        );
    }

    #[test]
    fn density_is_optional_and_unknown_is_not_replaced() {
        assert_eq!(density(None), Ok(()));
        assert_eq!(density(Some(7850.0)), Ok(()));
        assert_eq!(density(Some(MAX_DENSITY_KG_M3)), Ok(()));
        assert_eq!(density(Some(1e-9)), Ok(()));
    }

    #[test]
    fn density_rejections_have_their_codes() {
        assert_eq!(density(Some(f64::NAN)), Err(ErrorCode::NonFiniteNumber));
        assert_eq!(
            density(Some(f64::INFINITY)),
            Err(ErrorCode::NonFiniteNumber)
        );
        assert_eq!(density(Some(0.0)), Ok(()));
        assert_eq!(density(Some(-1.0)), Err(ErrorCode::InvalidArgument));
        assert_eq!(density(Some(1_000_000.5)), Err(ErrorCode::LimitExceeded));
    }
}
