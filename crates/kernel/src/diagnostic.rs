// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Typed F06 diagnostic subset for the rectangular plate and straight member builders.

use beamcraft_model::error::ErrorCode;

/// F06 stage at which evaluation stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Entry cancellation, before any validation.
    Validation,
    /// Submitted f64 finiteness.
    Finiteness,
    /// Submitted caps and derived local scale.
    Caps,
    /// Positivity and inclusive dimension minima.
    Ranges,
    /// Construction of the candidate mesh.
    Build,
    /// Closedness, orientation and numeric certification.
    Checks,
    /// Analytic volume before sealing.
    Quantities,
}

/// Stable geometry error code; this enum covers the plate and member builder failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    /// Submitted NaN or infinity.
    NonFinite,
    /// A zero or negative dimension.
    NotPositive,
    /// A positive dimension below the scaled minimum.
    BelowMinimum,
    /// A member axis of length zero or below the scaled minimum (F06 `GEO_AXIS_TOO_SHORT`).
    AxisTooShort,
    /// A dimension above its cap.
    AboveLimit,
    /// A derived numeric value could not be certified.
    NumericUncertain,
    /// A builder-produced mesh failed its self-check.
    SelfCheckFailed,
    /// Cancellation observed before sealing.
    Cancelled,
}

impl Code {
    /// Stable F06 wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NonFinite => "GEO_NON_FINITE",
            Self::NotPositive => "GEO_NOT_POSITIVE",
            Self::BelowMinimum => "GEO_BELOW_MINIMUM",
            Self::AxisTooShort => "GEO_AXIS_TOO_SHORT",
            Self::AboveLimit => "GEO_ABOVE_LIMIT",
            Self::NumericUncertain => "GEO_NUMERIC_UNCERTAIN",
            Self::SelfCheckFailed => "GEO_SELF_CHECK_FAILED",
            Self::Cancelled => "GEO_CANCELLED",
        }
    }

    /// Existing F04 code, without introducing a model-layer back-edge.
    pub const fn f04_code(self) -> ErrorCode {
        match self {
            Self::NonFinite => ErrorCode::NonFiniteNumber,
            Self::NotPositive | Self::BelowMinimum | Self::AxisTooShort => {
                ErrorCode::InvalidArgument
            }
            Self::AboveLimit => ErrorCode::LimitExceeded,
            Self::NumericUncertain | Self::SelfCheckFailed => ErrorCode::UnsupportedGeometry,
            Self::Cancelled => ErrorCode::Cancelled,
        }
    }
}

/// Fixed input schema token; never contains user text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// Rectangular width in millimetres.
    Width,
    /// Rectangular length in millimetres.
    Length,
    /// Plate thickness in millimetres.
    Thickness,
    /// Derived plate local size D.
    LocalSize,
    /// Constructed mesh.
    Mesh,
    /// Analytic volume.
    Volume,
    /// Host cancellation request.
    Cancellation,
    /// Member axis points.
    Axis,
    /// Member section width or height.
    Section,
    /// Member roll angle.
    Roll,
    /// Member section placement offsets (P14).
    SectionPlacement,
}

impl Field {
    /// Fixed ASCII token, shorter than F06's 64-byte bound.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Width => "width_mm",
            Self::Length => "length_mm",
            Self::Thickness => "thickness_mm",
            Self::LocalSize => "local_size_mm",
            Self::Mesh => "mesh",
            Self::Volume => "volume_mm3",
            Self::Cancellation => "cancellation",
            Self::Axis => "axis_points_mm",
            Self::Section => "profile",
            Self::Roll => "roll_rad",
            Self::SectionPlacement => "section_placement",
        }
    }
}

/// Safe diagnostic with a stable code, schema field and stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Diagnostic {
    /// Geometry failure code; its F04 mapping is [`Code::f04_code`].
    pub code: Code,
    /// Fixed schema token identifying the failing quantity.
    pub field: Field,
    /// First failing evaluation stage.
    pub stage: Stage,
}

/// Failed call, containing every failure of the first failing stage (at most 3).
///
/// It has no candidate geometry, quantities or key. No authored state is mutated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub(crate) diagnostics: [Option<Diagnostic>; 3],
}

impl Failure {
    /// Diagnostics in width, length, thickness order, or one derived failure.
    pub fn diagnostics(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().flatten()
    }

    /// F06 overall status, determined by the first diagnostic.
    pub fn status(&self) -> &'static str {
        match self.diagnostics().next().map(|d| d.code) {
            Some(Code::AboveLimit) => "limit_exceeded",
            Some(Code::Cancelled) => "cancelled",
            Some(Code::NumericUncertain | Code::SelfCheckFailed) => "unsupported",
            _ => "invalid_input",
        }
    }

    /// Shared with the steel crate.
    pub fn one(code: Code, field: Field, stage: Stage) -> Self {
        Self {
            diagnostics: [Some(Diagnostic { code, field, stage }), None, None],
        }
    }
}

impl core::fmt::Display for Failure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("rectangular plate evaluation failed")
    }
}

impl core::error::Error for Failure {}
