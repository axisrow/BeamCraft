// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D01: admitted intent for one associative member view and one projected linear dimension.
//!
//! The contract holds semantic references, never renderer indices or cached successful values.
//! Geometry resolution, USD persistence and publication are separate operations. A storage DTO
//! must enter through [`Drawing::admit`] before it can authorize any of them.

pub mod pdf;
pub mod print;
pub mod projection;
pub mod svg;

use crate::query::feature::{BodyKind, FeatureId, FeatureKind};
use beamcraft_model::command::{CommandError, label_is_valid};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{DimensionId, DrawingId, DrawingViewId, ElementId};

/// Version of the first closed drawing intent contract (distinct from the model schema).
pub const CONTRACT_VERSION: &str = "beamcraft-drawing/1";
/// Largest paper-space coordinate or signed dimension offset in millimetres.
pub const MAX_PAPER_MM: f64 = 10_000.0;
/// Largest scale denominator; the first slice admits scales from 1:1 through 1:10000.
pub const MAX_SCALE_DENOMINATOR: f64 = 10_000.0;
/// Default paper scale: one paper millimetre represents 100 model millimetres.
pub const DEFAULT_SCALE_DENOMINATOR: f64 = 100.0;

pub use beamcraft_model::drawing::{Freshness, LiveSource, SourceModelPin};

/// Fixed orthographic bases in a right-handed, Z-up model. Positive paper Y points up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plane {
    /// Model X goes right and model Z goes up.
    Front,
    /// Model X goes right and model Y goes up.
    Top,
    /// Model Y goes right and model Z goes up.
    Right,
}

impl Plane {
    /// Exact lowercase wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Front => "front",
            Self::Top => "top",
            Self::Right => "right",
        }
    }

    /// Parses a fixed basis; arbitrary camera matrices are outside this contract.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "front" => Some(Self::Front),
            "top" => Some(Self::Top),
            "right" => Some(Self::Right),
            _ => None,
        }
    }
}

/// One view's authored settings; scale never changes a dimension's model-unit value.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewSpec {
    /// Stable identity retained through refresh.
    pub id: DrawingViewId,
    /// Explicit projection basis.
    pub plane: Plane,
    /// Paper/model scale denominator (1 through 10000, default supplied by the caller is 100).
    pub scale_denominator: f64,
    /// Paper-space view origin in millimetres, each component within 10000 in magnitude.
    pub paper_origin_mm: [f64; 2],
}

/// One aligned projected linear dimension. Its endpoints must be vertices of the selected member.
#[derive(Debug, Clone, PartialEq)]
pub struct DimensionSpec {
    /// Stable dimension identity retained through refresh.
    pub id: DimensionId,
    /// Full semantic references in caller-chosen endpoint order.
    pub anchors: [FeatureId; 2],
    /// Signed perpendicular distance from the projected interval in paper millimetres.
    pub paper_offset_mm: f64,
    /// Decimal places for the millimetre label, from zero through three.
    pub decimal_places: u8,
}

/// Untrusted typed intent; use [`Drawing::admit`] before resolution or writing.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawingSpec {
    /// Stable document identity.
    pub id: DrawingId,
    /// Label under the model's existing bounded label rule.
    pub name: String,
    /// Exact captured source identity; the adapter must verify the source manifest and contents.
    pub source: SourceModelPin,
    /// The sole selected straight member; copies have different identities and never relink.
    pub member: ElementId,
    /// The sole view.
    pub view: ViewSpec,
    /// The sole dimension.
    pub dimension: DimensionSpec,
}

/// Validated immutable intent. Admission grants no claim that the referenced geometry exists.
#[derive(Debug, Clone, PartialEq)]
pub struct Drawing(DrawingSpec);

fn invalid(field: &'static str, message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::InvalidArgument, field, message)
}

fn finite_range(value: f64, min: f64, max: f64, field: &'static str) -> Result<(), CommandError> {
    if !value.is_finite() {
        return Err(CommandError::new(
            ErrorCode::NonFiniteNumber,
            field,
            "number must be finite",
        ));
    }
    if value < min || value > max {
        return Err(invalid(
            field,
            "number is outside the drawing contract's range",
        ));
    }
    Ok(())
}

impl Drawing {
    /// Admits one bounded document and its qualified semantic references without any geometry work.
    /// Duplicate drawing/view/dimension UUIDs, invalid labels, foreign/non-vertex references,
    /// nonfinite numbers and out-of-range paper settings refuse with no partial result.
    pub fn admit(spec: DrawingSpec) -> Result<Self, CommandError> {
        let ids = [
            spec.id.uuid(),
            spec.view.id.uuid(),
            spec.dimension.id.uuid(),
        ];
        if ids
            .iter()
            .enumerate()
            .any(|(i, id)| ids.iter().skip(i + 1).any(|other| id == other))
        {
            return Err(CommandError::new(
                ErrorCode::DuplicateId,
                "drawing_ids",
                "document object UUIDs must be distinct",
            ));
        }
        if !label_is_valid(&spec.name) {
            return Err(invalid(
                "name",
                "drawing name must satisfy the bounded label rule",
            ));
        }
        finite_range(
            spec.view.scale_denominator,
            1.0,
            MAX_SCALE_DENOMINATOR,
            "scale_denominator",
        )?;
        for value in spec.view.paper_origin_mm {
            finite_range(value, -MAX_PAPER_MM, MAX_PAPER_MM, "paper_origin_mm")?;
        }
        finite_range(
            spec.dimension.paper_offset_mm,
            -MAX_PAPER_MM,
            MAX_PAPER_MM,
            "paper_offset_mm",
        )?;
        if spec.dimension.decimal_places > 3 {
            return Err(invalid(
                "decimal_places",
                "use zero through three decimal places",
            ));
        }
        for anchor in &spec.dimension.anchors {
            if anchor.element() != spec.member
                || anchor.body() != BodyKind::Member
                || anchor.kind() != FeatureKind::Vertex
            {
                return Err(invalid(
                    "anchors",
                    "dimension anchors must be qualified vertices of the selected member",
                ));
            }
        }
        // Equal semantic endpoints are invalid intent. Distinct endpoints that collapse only in
        // the chosen projection remain valid intent and resolve as explicitly unresolved later.
        let [first, second] = &spec.dimension.anchors;
        if first == second {
            return Err(invalid(
                "anchors",
                "dimension endpoints must be distinct semantic references",
            ));
        }
        Ok(Self(spec))
    }

    /// Borrows the validated intent; callers cannot mutate it around admission.
    pub fn spec(&self) -> &DrawingSpec {
        &self.0
    }

    /// Explicit refresh replaces only the captured source, retaining object IDs and references.
    /// The adapter is responsible for verifying/capturing this pin before persistence.
    pub fn refreshed(&self, source: SourceModelPin) -> Self {
        let mut spec = self.0.clone();
        spec.source = source;
        Self(spec)
    }
}
