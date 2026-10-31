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
use crate::limits::{
    DECLARED_RELATIVE_TOLERANCE, DECLARED_SECTION_AREA_TOLERANCE, DIMENSION_CAP_MM, MAX_AREA_MM2,
    MAX_DENSITY_KG_M3, MAX_MASS_PER_METER_KG, MAX_SURFACE_PER_METER_M2,
};
use crate::section::SectionShape;
use beamcraft_geom::tolerance::ABSOLUTE_FLOOR_MM;

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

fn declared_label(value: &Option<String>, field: &'static str) -> Result<(), CommandError> {
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

/// Declared section properties of a profile definition (P02.2): the catalogue identity and the
/// numbers a bill of materials reads. Every value is optional and stays unknown when absent;
/// nothing is inferred from the designation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SectionProperties {
    /// Designation as printed in the cited standard, a label only.
    pub designation: Option<String>,
    /// Standard that defines the designation, a label only.
    pub standard: Option<String>,
    /// Declared cross-section area in mm2.
    pub area_mm2: Option<f64>,
    /// Declared catalogue mass per metre in kg/m. Used for the nominal mass of a member.
    pub mass_per_meter_kg: Option<f64>,
    /// Declared surface area per metre of length in m2/m.
    pub surface_per_meter_m2: Option<f64>,
    /// Readable name shown in listings and used to select the definition by name (A07.2). Part
    /// of the definition content, so another name is another digest. A label only.
    pub display_name: Option<String>,
}

impl SectionProperties {
    /// Admits the declared labels and numbers: labels by the element-label rule, numbers finite
    /// (`NON_FINITE_NUMBER`), greater than zero (`INVALID_ARGUMENT`) and under their caps
    /// (`LIMIT_EXCEEDED`). A committed definition is re-admitted with this before its declared
    /// area is used for a quantity.
    pub fn validate(&self) -> Result<(), CommandError> {
        declared_label(&self.designation, "designation")?;
        declared_label(&self.standard, "standard")?;
        declared_label(&self.display_name, "display_name")?;
        for (value, cap, field) in [
            (self.area_mm2, MAX_AREA_MM2, "area_mm2"),
            (
                self.mass_per_meter_kg,
                MAX_MASS_PER_METER_KG,
                "mass_per_meter_kg",
            ),
            (
                self.surface_per_meter_m2,
                MAX_SURFACE_PER_METER_M2,
                "surface_per_meter_m2",
            ),
        ] {
            let Some(value) = value else { continue };
            if !value.is_finite() {
                return Err(CommandError::new(
                    ErrorCode::NonFiniteNumber,
                    field,
                    "value is not finite",
                )
                .with_rule("DV-ATTR-003"));
            }
            if value > cap {
                return Err(CommandError::new(
                    ErrorCode::LimitExceeded,
                    field,
                    "value exceeds its cap",
                ));
            }
            if value <= 0.0 {
                return Err(CommandError::new(
                    ErrorCode::InvalidArgument,
                    field,
                    "value must be greater than zero",
                ));
            }
        }
        Ok(())
    }
}

/// The first-scope parametric profile: a centred rectangle section (F06 section 2).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RectangleProfileSpec {
    /// Section width in millimetres (along the rolled local X).
    pub width_mm: f64,
    /// Section height in millimetres (along the rolled local Y).
    pub height_mm: f64,
    /// Declared catalogue identity and section numbers; all optional.
    pub section: SectionProperties,
}

impl RectangleProfileSpec {
    /// Admits both dimensions: finite (`NON_FINITE_NUMBER`), at least the absolute geometric
    /// floor of 0.001 mm (`INVALID_ARGUMENT`), then at most 1,000,000 mm (`LIMIT_EXCEEDED`).
    /// This makes negative finite values invalid regardless of magnitude. Declared section
    /// numbers are admitted the same way and, for a
    /// rectangle, must agree with the family's own formula within 0.1 percent: the area with
    /// `width * height` and the surface per metre with the perimeter (`INVALID_ARGUMENT`).
    pub fn validate(&self) -> Result<(), CommandError> {
        let dimensions = [self.width_mm, self.height_mm];
        if dimensions.iter().any(|v| !v.is_finite()) {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "profile",
                "value is not finite",
            )
            .with_rule("DV-ATTR-003"));
        }
        if dimensions.iter().any(|v| *v < ABSOLUTE_FLOOR_MM) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "profile",
                "section dimension is below the geometric floor",
            ));
        }
        if dimensions.iter().any(|v| *v > DIMENSION_CAP_MM) {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "profile",
                "section dimension exceeds the dimension cap",
            )
            .with_rule("DV-LIM-001"));
        }
        self.section.validate()?;
        let agrees = |declared: f64, formula: f64| {
            (declared - formula).abs() <= DECLARED_RELATIVE_TOLERANCE * formula
        };
        if let Some(area) = self.section.area_mm2
            && !agrees(area, self.width_mm * self.height_mm)
        {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "area_mm2",
                "declared area differs from width times height",
            ));
        }
        if let Some(surface) = self.section.surface_per_meter_m2
            && !agrees(surface, 2.0 * (self.width_mm + self.height_mm) / 1000.0)
        {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "surface_per_meter_m2",
                "declared surface differs from the perimeter",
            ));
        }
        Ok(())
    }
}

/// A profile of a section family beyond the rectangle (A07.1): a parametric family or an
/// authored outline, with optional declared section numbers.
#[derive(Debug, Clone, PartialEq)]
pub struct SectionProfileSpec {
    /// The section shape.
    pub shape: SectionShape,
    /// Declared catalogue identity and section numbers; all optional.
    pub section: SectionProperties,
}

impl SectionProfileSpec {
    /// Admits the shape and the declared values. The declared area is a catalogue value and is
    /// never refused for disagreeing with the outline; [`Self::declared_area_warning`] reports it.
    pub fn validate(&self) -> Result<(), CommandError> {
        self.shape.validate()?;
        self.section.validate()
    }

    /// Compares the declared area, if any, with the EXACT area of the section in mm2 (P-Mass:
    /// the closed-form area of [`crate::section::SectionShape::exact_area_mm2`], supplied by the
    /// caller; never the polygon of the mesh). A relative gap above 1 percent
    /// (`DECLARED_SECTION_AREA_TOLERANCE`) is a warning, never a refusal: owner decision D3
    /// (2026-10-18), catalogue values are authoritative. No declared area, or an exact area
    /// that is not a positive finite number, gives no warning.
    pub fn declared_area_warning(&self, exact_area_mm2: f64) -> Option<DeclaredAreaWarning> {
        let declared = self.section.area_mm2?;
        if !exact_area_mm2.is_finite() || exact_area_mm2 <= 0.0 {
            return None;
        }
        let relative_difference = (declared - exact_area_mm2).abs() / exact_area_mm2;
        (relative_difference > DECLARED_SECTION_AREA_TOLERANCE).then_some(DeclaredAreaWarning {
            declared_mm2: declared,
            kernel_mm2: exact_area_mm2,
            relative_difference,
            threshold: DECLARED_SECTION_AREA_TOLERANCE,
        })
    }
}

/// A declared section area that differs from the exact area of the section by more than the
/// threshold. Informational: the definition is created and keeps the declared value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeclaredAreaWarning {
    /// The declared (catalogue) area in mm2.
    pub declared_mm2: f64,
    /// The exact area of the section in mm2 (the name is kept from the first slice, where the
    /// kernel's polygon was the reference; the wire key is `kernel_mm2`).
    pub kernel_mm2: f64,
    /// `|declared - exact| / exact`.
    pub relative_difference: f64,
    /// The relative threshold that was exceeded (0.01).
    pub threshold: f64,
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

    fn rectangle(width_mm: f64, height_mm: f64) -> Result<(), ErrorCode> {
        RectangleProfileSpec {
            width_mm,
            height_mm,
            section: SectionProperties::default(),
        }
        .validate()
        .map_err(|e| e.code)
    }

    fn declared(section: SectionProperties) -> Result<(), ErrorCode> {
        RectangleProfileSpec {
            width_mm: 200.0,
            height_mm: 300.0,
            section,
        }
        .validate()
        .map_err(|e| e.code)
    }

    #[test]
    fn declared_section_numbers_and_labels_are_admitted_or_refused_by_code() {
        let ok = SectionProperties {
            designation: Some("RECT 200x300".into()),
            standard: Some("Synthetic".into()),
            area_mm2: Some(60_000.0),
            mass_per_meter_kg: Some(471.0),
            surface_per_meter_m2: Some(1.0),
            ..SectionProperties::default()
        };
        assert_eq!(declared(ok.clone()), Ok(()));
        let with = |f: fn(&mut SectionProperties)| {
            let mut s = ok.clone();
            f(&mut s);
            declared(s)
        };
        assert_eq!(
            with(|s| s.area_mm2 = Some(60_100.0)),
            Err(ErrorCode::InvalidArgument)
        );
        assert_eq!(
            with(|s| s.surface_per_meter_m2 = Some(1.1)),
            Err(ErrorCode::InvalidArgument)
        );
        assert_eq!(
            with(|s| s.mass_per_meter_kg = Some(f64::NAN)),
            Err(ErrorCode::NonFiniteNumber)
        );
        assert_eq!(
            with(|s| s.mass_per_meter_kg = Some(0.0)),
            Err(ErrorCode::InvalidArgument)
        );
        assert_eq!(
            with(|s| s.mass_per_meter_kg = Some(2e6)),
            Err(ErrorCode::LimitExceeded)
        );
        assert_eq!(
            with(|s| s.designation = Some(String::new())),
            Err(ErrorCode::InvalidArgument)
        );
        assert_eq!(
            with(|s| s.display_name = Some("a\nb".into())),
            Err(ErrorCode::InvalidArgument)
        );
        assert_eq!(
            with(|s| s.display_name = Some("RECT 200x300".into())),
            Ok(())
        );
        assert_eq!(
            with(|s| s.standard = Some("a\nb".into())),
            Err(ErrorCode::InvalidArgument)
        );
    }

    #[test]
    fn a_declared_section_area_beyond_one_percent_is_a_warning_not_a_refusal() {
        use crate::section::{ParametricSection, SectionFamily};
        let spec = |area: Option<f64>| SectionProfileSpec {
            shape: SectionShape::Parametric(
                ParametricSection::new(SectionFamily::Chs, vec![100.0, 5.0]).unwrap(),
            ),
            section: SectionProperties {
                area_mm2: area,
                ..SectionProperties::default()
            },
        };
        // The spec itself is admitted whatever the declared area is.
        assert!(spec(Some(1.0)).validate().is_ok());
        assert_eq!(spec(None).declared_area_warning(1000.0), None);
        assert_eq!(spec(Some(1009.0)).declared_area_warning(1000.0), None);
        assert_eq!(spec(Some(991.0)).declared_area_warning(1000.0), None);
        let warning = spec(Some(1100.0)).declared_area_warning(1000.0).unwrap();
        assert_eq!(warning.declared_mm2, 1100.0);
        assert_eq!(warning.kernel_mm2, 1000.0);
        assert!((warning.relative_difference - 0.1).abs() < 1e-12);
        assert_eq!(warning.threshold, 0.01);
        assert!(spec(Some(989.0)).declared_area_warning(1000.0).is_some());
        // An outline area that cannot be compared gives no warning.
        assert_eq!(spec(Some(1000.0)).declared_area_warning(f64::NAN), None);
        assert_eq!(spec(Some(1000.0)).declared_area_warning(0.0), None);
        // Malformed declared values are still refused.
        for bad in [f64::NAN, 0.0, -5.0, 2e12] {
            assert!(spec(Some(bad)).validate().is_err(), "{bad}");
        }
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

    #[test]
    fn rectangle_dimensions_follow_the_f04_caps_and_floor() {
        assert_eq!(rectangle(200.0, 300.0), Ok(()));
        assert_eq!(rectangle(0.001, 1_000_000.0), Ok(()));
        assert_eq!(rectangle(0.0009, 300.0), Err(ErrorCode::InvalidArgument));
        assert_eq!(rectangle(200.0, 0.0), Err(ErrorCode::InvalidArgument));
        assert_eq!(rectangle(-200.0, 300.0), Err(ErrorCode::InvalidArgument));
        assert_eq!(
            rectangle(-3_000_000.0, 300.0),
            Err(ErrorCode::InvalidArgument)
        );
        assert_eq!(rectangle(-1e308, 300.0), Err(ErrorCode::InvalidArgument));
        assert_eq!(rectangle(1e308, 300.0), Err(ErrorCode::LimitExceeded));
        assert_eq!(rectangle(200.0, 1_000_001.0), Err(ErrorCode::LimitExceeded));
        assert_eq!(rectangle(f64::NAN, 300.0), Err(ErrorCode::NonFiniteNumber));
        assert_eq!(
            rectangle(200.0, f64::NEG_INFINITY),
            Err(ErrorCode::NonFiniteNumber)
        );
    }
}
