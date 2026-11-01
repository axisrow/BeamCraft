// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Typed F06 diagnostics for profile admission, extrusion and curve generation.
//!
//! The `GEO_` tokens and their F04 mapping are exactly those of F06 section 3;
//! this module only adds the contour-related codes that G02's plate builder did
//! not need. No record carries user text, a coordinate value or a path.

use beamcraft_model::error::ErrorCode;

pub use crate::diagnostic::Stage;

/// Largest diagnostic list of one failed call (F06 section 3).
pub const MAX_DIAGNOSTICS: usize = 32;

/// Stable F06 `GEO_` code used by the profile builders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileCode {
    /// Submitted NaN or infinity.
    NonFinite,
    /// A zero or negative dimension.
    NotPositive,
    /// A positive dimension below the scaled minimum.
    BelowMinimum,
    /// A count, coordinate or dimension above its cap.
    AboveLimit,
    /// An invalid contour under F04 section 3.
    ContourInvalid,
    /// A valid contour outside the supported scope (for example a nested island).
    UnsupportedContour,
    /// A derived vertex, triangle or byte count above the F06 section 6 cap.
    BufferLimit,
    /// A derived face, edge or feature count above the F06 section 6 cap.
    TopologyLimit,
    /// A numeric certificate or exact predicate could not decide.
    NumericUncertain,
    /// The builder's own result failed its section 6 self-checks.
    SelfCheckFailed,
    /// Cancellation observed before sealing.
    Cancelled,
}

impl ProfileCode {
    /// Stable F06 wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NonFinite => "GEO_NON_FINITE",
            Self::NotPositive => "GEO_NOT_POSITIVE",
            Self::BelowMinimum => "GEO_BELOW_MINIMUM",
            Self::AboveLimit => "GEO_ABOVE_LIMIT",
            Self::ContourInvalid => "GEO_CONTOUR_INVALID",
            Self::UnsupportedContour => "GEO_UNSUPPORTED_CONTOUR",
            Self::BufferLimit => "GEO_BUFFER_LIMIT",
            Self::TopologyLimit => "GEO_TOPOLOGY_LIMIT",
            Self::NumericUncertain => "GEO_NUMERIC_UNCERTAIN",
            Self::SelfCheckFailed => "GEO_SELF_CHECK_FAILED",
            Self::Cancelled => "GEO_CANCELLED",
        }
    }

    /// Existing F04 code, as mapped by F06 section 3.
    pub const fn f04_code(self) -> ErrorCode {
        match self {
            Self::NonFinite => ErrorCode::NonFiniteNumber,
            Self::NotPositive | Self::BelowMinimum | Self::ContourInvalid => {
                ErrorCode::InvalidArgument
            }
            Self::AboveLimit | Self::BufferLimit | Self::TopologyLimit => ErrorCode::LimitExceeded,
            Self::UnsupportedContour | Self::NumericUncertain | Self::SelfCheckFailed => {
                ErrorCode::UnsupportedGeometry
            }
            Self::Cancelled => ErrorCode::Cancelled,
        }
    }
}

/// Fixed schema token naming the failing quantity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileField {
    /// Extrusion thickness.
    Thickness,
    /// Outer contour points.
    OuterContour,
    /// Cavity contour points.
    CavityContour,
    /// Number of cavities.
    CavityCount,
    /// Total number of contour points.
    TotalPoints,
    /// Derived local size D.
    LocalSize,
    /// Curve generator parameter.
    Curve,
    /// Constructed mesh.
    Mesh,
    /// Analytic volume.
    Volume,
    /// Host cancellation request.
    Cancellation,
}

impl ProfileField {
    /// Fixed ASCII token, shorter than F06's 64-byte bound.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Thickness => "thickness_mm",
            Self::OuterContour => "outer_contour_mm",
            Self::CavityContour => "cavity_contour_mm",
            Self::CavityCount => "cavity_count",
            Self::TotalPoints => "total_points",
            Self::LocalSize => "local_size_mm",
            Self::Curve => "curve",
            Self::Mesh => "mesh",
            Self::Volume => "volume_mm3",
            Self::Cancellation => "cancellation",
        }
    }
}

/// Fixed classification of a contour or generator defect, for precise tests and UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Defect {
    /// No finer classification applies.
    None,
    /// A contour has fewer than three points.
    TooFewPoints,
    /// An edge (including a repeated point or the closing edge) is below the floor.
    EdgeTooShort,
    /// The contour has exactly zero signed area.
    ZeroArea,
    /// The contour area is below the scaled minimum.
    AreaTooSmall,
    /// The outer contour is clockwise or a cavity is counter-clockwise.
    WrongWinding,
    /// Two non-adjacent edges of one contour intersect, touch or overlap.
    SelfIntersection,
    /// Two adjacent edges fold back onto each other.
    Backtracking,
    /// Two non-adjacent edges of one contour are closer than the floor.
    NarrowFeature,
    /// Edges of two different contours intersect, touch or overlap.
    ContoursIntersect,
    /// Edges of two different contours are closer than the floor.
    ContoursTooClose,
    /// A cavity lies outside the outer contour.
    CavityOutside,
    /// A cavity lies inside another cavity (an island inside a hole).
    CavityNested,
    /// A curve generator input is invalid.
    CurveInput,
    /// The requested sagitta bound needs more segments than the maximum.
    SagittaUnreachable,
    /// The bounded triangulation work budget was exhausted.
    WorkBudget,
}

/// Safe diagnostic with a stable code, schema field, stage and fixed location.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileDiagnostic {
    /// Geometry failure code; its F04 mapping is [`ProfileCode::f04_code`].
    pub code: ProfileCode,
    /// Fixed schema token identifying the failing quantity.
    pub field: ProfileField,
    /// First failing evaluation stage.
    pub stage: Stage,
    /// Finer classification of the defect.
    pub defect: Defect,
    /// Cavity index in input order, or `None` for the outer contour or a global item.
    pub cavity: Option<u32>,
    /// Point or edge index inside the contour, in input order, when one applies.
    pub index: Option<u32>,
}

/// Failed call: every failure of the first failing stage, at most 32, in input order.
///
/// It carries no candidate geometry, quantity or key; nothing is partially built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileFailure {
    pub(crate) diagnostics: Vec<ProfileDiagnostic>,
}

impl ProfileFailure {
    /// Diagnostics in input field order.
    pub fn diagnostics(&self) -> &[ProfileDiagnostic] {
        &self.diagnostics
    }

    /// F06 overall status, determined by the first diagnostic.
    pub fn status(&self) -> &'static str {
        match self.diagnostics.first().map(|d| d.code) {
            Some(
                ProfileCode::AboveLimit | ProfileCode::BufferLimit | ProfileCode::TopologyLimit,
            ) => "limit_exceeded",
            Some(ProfileCode::Cancelled) => "cancelled",
            Some(
                ProfileCode::UnsupportedContour
                | ProfileCode::NumericUncertain
                | ProfileCode::SelfCheckFailed,
            ) => "unsupported",
            _ => "invalid_input",
        }
    }

    pub(crate) fn one(
        code: ProfileCode,
        field: ProfileField,
        stage: Stage,
        defect: Defect,
        cavity: Option<u32>,
        index: Option<u32>,
    ) -> Self {
        Self {
            diagnostics: vec![ProfileDiagnostic {
                code,
                field,
                stage,
                defect,
                cavity,
                index,
            }],
        }
    }

    /// Shared with the steel crate.
    pub fn global(code: ProfileCode, field: ProfileField, stage: Stage) -> Self {
        Self::one(code, field, stage, Defect::None, None, None)
    }
}

impl core::fmt::Display for ProfileFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("profile evaluation failed")
    }
}

impl core::error::Error for ProfileFailure {}

/// Bounded diagnostic collector for one stage.
#[derive(Default)]
pub(crate) struct Collector {
    items: Vec<ProfileDiagnostic>,
}

impl Collector {
    pub(crate) fn push(&mut self, diagnostic: ProfileDiagnostic) {
        if self.items.len() < MAX_DIAGNOSTICS {
            self.items.push(diagnostic);
        }
    }

    pub(crate) fn has_any(&self) -> bool {
        !self.items.is_empty()
    }

    pub(crate) fn into_result(self) -> Result<(), ProfileFailure> {
        if self.items.is_empty() {
            Ok(())
        } else {
            Err(ProfileFailure {
                diagnostics: self.items,
            })
        }
    }
}
