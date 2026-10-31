// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D07 drawn objects owned by a drawing document.
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
/// Largest number of vertices of one drawn polyline or hatch outline.
pub const MAX_POLYLINE_POINTS: usize = 16;
/// Largest number of layers in one document.
pub const MAX_LAYERS: usize = 16;
/// Largest byte length of one drawn text string.
pub const MAX_TEXT_BYTES: usize = 128;
/// Largest byte length of one coordinate's canonical decimal form. A pathological binary64
/// (say `5e-324`) prints as hundreds of characters and would overflow any fixed document byte
/// cap; a coordinate whose shortest round-trip form exceeds this bound is refused at admission
/// exactly like an oversized text string is.
pub const MAX_NUMBER_TEXT_BYTES: usize = 24;

/// One complete turn in radians; the bound for an admitted arc angle magnitude.
pub const MAX_ARC_TURN: f64 = std::f64::consts::TAU;

/// The fill of a drawn hatch. These are paper patterns only; they never carry model material
/// or component semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HatchPattern {
    /// A flat opaque fill.
    Solid,
    /// Single parallel lines in one direction.
    Diagonal,
    /// Two crossing line directions.
    Crosshatch,
}

impl HatchPattern {
    /// Stable lowercase wire token, also the persisted `drawing:pattern` value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Solid => "solid",
            Self::Diagonal => "diagonal",
            Self::Crosshatch => "crosshatch",
        }
    }

    /// Parses a fixed pattern token.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "solid" => Some(Self::Solid),
            "diagonal" => Some(Self::Diagonal),
            "crosshatch" => Some(Self::Crosshatch),
            _ => None,
        }
    }
}

/// The fixed set of paper symbols a document can place. Placing is hand-authored intent; a
/// symbol resolves no model feature and carries no model mark semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaperSymbol {
    /// North arrow for the sheet orientation.
    North,
    /// Section-cut direction marker.
    Section,
    /// Detail view callout ring.
    Detail,
    /// Level/elevation marker.
    Elevation,
}

impl PaperSymbol {
    /// Stable lowercase wire token, also the persisted `drawing:symbol` value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::North => "north",
            Self::Section => "section",
            Self::Detail => "detail",
            Self::Elevation => "elevation",
        }
    }

    /// Parses a fixed symbol token.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "north" => Some(Self::North),
            "section" => Some(Self::Section),
            "detail" => Some(Self::Detail),
            "elevation" => Some(Self::Elevation),
            _ => None,
        }
    }
}

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
    /// Free text anchored at a point: a label the drawing owns and the model never carries.
    Text {
        /// Anchor in paper millimetres.
        anchor: [f64; 2],
        /// Non-empty UTF-8 without control characters, at most [`MAX_TEXT_BYTES`] bytes.
        text: String,
        /// Glyph height in paper millimetres, greater than zero.
        height: f64,
        /// Counter-clockwise rotation from paper X, magnitude at most one turn.
        rotation: f64,
    },
    /// A closed hatched contour; the fill is a named paper pattern, never model material.
    Hatch {
        /// Contour vertices in drawn order; the last vertex closes to the first implicitly.
        outline: Vec<[f64; 2]>,
        /// The fill pattern.
        pattern: HatchPattern,
    },
    /// One instance of the fixed [`PaperSymbol`] set, placed by hand.
    Symbol {
        /// Anchor in paper millimetres.
        anchor: [f64; 2],
        /// The placed symbol.
        symbol: PaperSymbol,
        /// Positive uniform scale multiplier of the symbol's nominal paper size.
        scale: f64,
    },
    /// A linear dimension measured between two drawn paper points. It resolves nothing: the
    /// label is the paper interval itself, and no model feature is bound or consulted.
    FreeDimension {
        /// First measured point.
        from: [f64; 2],
        /// Second measured point, distinct from `from`.
        to: [f64; 2],
        /// Signed perpendicular offset of the dimension line in paper millimetres.
        offset: f64,
        /// Decimal places for the millimetre label, zero through three.
        decimal_places: u8,
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
            Self::Text { .. } => "text",
            Self::Hatch { .. } => "hatch",
            Self::Symbol { .. } => "symbol",
            Self::FreeDimension { .. } => "free_dimension",
        }
    }

    /// The kind-specific string payload and its persisted attribute name: `drawing:text`,
    /// `drawing:pattern` or `drawing:symbol`. Geometry without a string payload returns `None`.
    pub fn string_payload(&self) -> Option<(&'static str, &str)> {
        match self {
            Self::Text { text, .. } => Some(("drawing:text", text.as_str())),
            Self::Hatch { pattern, .. } => Some(("drawing:pattern", pattern.as_str())),
            Self::Symbol { symbol, .. } => Some(("drawing:symbol", symbol.as_str())),
            _ => None,
        }
    }

    /// The persisted string-payload attribute name for one kind token, or `None` for a kind
    /// that carries only coordinates.
    pub fn payload_attribute(kind: &str) -> Option<&'static str> {
        match kind {
            "text" => Some("drawing:text"),
            "hatch" => Some("drawing:pattern"),
            "symbol" => Some("drawing:symbol"),
            _ => None,
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
            Self::Text {
                anchor,
                height,
                rotation,
                ..
            } => {
                push(&mut out, anchor);
                out.push(height);
                out.push(rotation);
            }
            Self::Hatch { outline, .. } => {
                for point in outline {
                    push(&mut out, point);
                }
            }
            Self::Symbol { anchor, scale, .. } => {
                push(&mut out, anchor);
                out.push(scale);
            }
            Self::FreeDimension {
                from,
                to,
                offset,
                decimal_places,
            } => {
                push(&mut out, from);
                push(&mut out, to);
                out.push(offset);
                out.push(f64::from(decimal_places));
            }
        }
        out
    }

    /// Rebuilds geometry from a kind token, its flat coordinate payload and the optional
    /// string payload. Used by the closed intent reader and the CLI front end; validation
    /// still runs in [`validate_geometry`].
    pub fn from_parts(kind: &str, values: &[f64], payload: Option<&str>) -> Option<Self> {
        if Self::payload_attribute(kind).is_some() != payload.is_some() {
            return None;
        }
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
        let decimal_places = |value: f64| match value {
            0.0 => Some(0),
            1.0 => Some(1),
            2.0 => Some(2),
            3.0 => Some(3),
            _ => None,
        };
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
            "text" => Some(Self::Text {
                anchor: point(values, 0)?,
                text: payload?.to_owned(),
                height: number(2)?,
                rotation: number(3)?,
            }),
            "hatch" => {
                let pattern = HatchPattern::parse(payload?)?;
                if values.len() < 6 || !values.len().is_multiple_of(2) {
                    return None;
                }
                let outline = values
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|&pair| Some(pair))
                    .collect::<Option<Vec<_>>>()?;
                Some(Self::Hatch { outline, pattern })
            }
            "symbol" => Some(Self::Symbol {
                anchor: point(values, 0)?,
                symbol: PaperSymbol::parse(payload?)?,
                scale: number(2)?,
            }),
            "free_dimension" => Some(Self::FreeDimension {
                from: point(values, 0)?,
                to: point(values, 2)?,
                offset: number(4)?,
                decimal_places: decimal_places(number(5)?)?,
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

/// The stroke pattern of a drawn object. These are paper line styles only; they never carry
/// model semantics such as hidden-edge state resolved from the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineType {
    /// Unbroken stroke.
    Continuous,
    /// Even alternating strokes and gaps.
    Dashed,
    /// Dots without strokes.
    Dotted,
    /// Alternating long strokes, dots and gaps.
    DashDot,
}

impl LineType {
    /// Stable lowercase wire token, also the persisted `drawing:linetype` value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Continuous => "continuous",
            Self::Dashed => "dashed",
            Self::Dotted => "dotted",
            Self::DashDot => "dashdot",
        }
    }

    /// Parses a fixed line-type token.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "continuous" => Some(Self::Continuous),
            "dashed" => Some(Self::Dashed),
            "dotted" => Some(Self::Dotted),
            "dashdot" => Some(Self::DashDot),
            _ => None,
        }
    }
}

/// One document-owned layer: a named, toggleable grouping that drawn objects join by
/// reference. A layer holds no model semantics and no geometry of its own; hiding it is
/// paper visibility only and never touches the engineering model.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerSpec {
    /// Stable layer identity, distinct from every other document object UUID.
    pub id: DraftingEntityId,
    /// Layer name under the model's existing bounded label rule.
    pub name: String,
    /// Whether drawn objects on this layer are visible on the paper.
    pub visible: bool,
}

/// One drawn object: identity plus paper geometry plus its layer and stroke assignment.
#[derive(Debug, Clone, PartialEq)]
pub struct DraftingSpec {
    /// Stable entity identity, distinct from every other document object UUID.
    pub id: DraftingEntityId,
    /// The drawn geometry.
    pub geometry: DraftingGeometry,
    /// The document-defined layer this object sits on; admission refuses a layer the
    /// document does not define.
    pub layer: DraftingEntityId,
    /// The stroke pattern of this object.
    pub line_type: LineType,
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
        bounded_form(value)?;
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
            bounded_form(*radius)?;
            if start.abs() > MAX_ARC_TURN || end.abs() > MAX_ARC_TURN {
                return Err(invalid(
                    "drafting",
                    "arc angles must stay within one turn in magnitude",
                ));
            }
            bounded_form(*start)?;
            bounded_form(*end)?;
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
            bounded_form(*radius)?;
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
        DraftingGeometry::Text {
            anchor,
            text,
            height,
            rotation,
        } => {
            finite_point(*anchor)?;
            if !height.is_finite() || !rotation.is_finite() {
                return Err(CommandError::new(
                    ErrorCode::NonFiniteNumber,
                    "drafting",
                    "number must be finite",
                ));
            }
            if *height <= 0.0 || *height > MAX_PAPER_MM {
                return Err(invalid(
                    "drafting",
                    "text height must be positive and within the drawing contract's range",
                ));
            }
            bounded_form(*height)?;
            if rotation.abs() > MAX_ARC_TURN {
                return Err(invalid(
                    "drafting",
                    "text rotation must stay within one turn in magnitude",
                ));
            }
            bounded_form(*rotation)?;
            if text.is_empty() || text.len() > MAX_TEXT_BYTES {
                return Err(invalid(
                    "drafting",
                    "text must be non-empty and within the byte cap",
                ));
            }
            if text.chars().any(char::is_control) {
                return Err(invalid("drafting", "text must not hold control characters"));
            }
        }
        DraftingGeometry::Hatch { outline, .. } => {
            if outline.len() < 3 || outline.len() > MAX_POLYLINE_POINTS {
                return Err(invalid(
                    "drafting",
                    "hatch outline needs three through sixteen vertices",
                ));
            }
            // The outline closes implicitly, so an equal final vertex would be a zero-length
            // closing segment exactly as an equal consecutive pair would.
            let mut previous: Option<[f64; 2]> = outline.last().copied();
            for point in outline {
                finite_point(*point)?;
                if previous == Some(*point) {
                    return Err(invalid(
                        "drafting",
                        "hatch outline segments must have positive length",
                    ));
                }
                previous = Some(*point);
            }
        }
        DraftingGeometry::Symbol { anchor, scale, .. } => {
            finite_point(*anchor)?;
            if !scale.is_finite() {
                return Err(CommandError::new(
                    ErrorCode::NonFiniteNumber,
                    "drafting",
                    "number must be finite",
                ));
            }
            if *scale <= 0.0 || *scale > MAX_PAPER_MM {
                return Err(invalid(
                    "drafting",
                    "symbol scale must be positive and within the drawing contract's range",
                ));
            }
            bounded_form(*scale)?;
        }
        DraftingGeometry::FreeDimension {
            from,
            to,
            offset,
            decimal_places,
        } => {
            finite_point(*from)?;
            finite_point(*to)?;
            if from == to {
                return Err(invalid(
                    "drafting",
                    "free dimension endpoints must be distinct",
                ));
            }
            finite_range_value(*offset, "drafting")?;
            // The u8 type already caps the count; the drawing contract's label precision is
            // the same zero-through-three rule the associative dimension uses.
            if *decimal_places > 3 {
                return Err(invalid(
                    "decimal_places",
                    "use zero through three decimal places",
                ));
            }
        }
    }
    Ok(())
}

/// Rejects a number whose canonical decimal form would overflow the document's byte budget.
fn bounded_form(value: f64) -> Result<(), CommandError> {
    if value.to_string().len() > MAX_NUMBER_TEXT_BYTES {
        return Err(invalid(
            "drafting",
            "number's canonical form is too long for the document byte cap",
        ));
    }
    Ok(())
}

/// Validates one finite value inside the drawing contract's signed paper range.
fn finite_range_value(value: f64, field: &'static str) -> Result<(), CommandError> {
    if !value.is_finite() {
        return Err(CommandError::new(
            ErrorCode::NonFiniteNumber,
            field,
            "number must be finite",
        ));
    }
    if value.abs() > MAX_PAPER_MM {
        return Err(invalid(
            field,
            "number is outside the drawing contract's range",
        ));
    }
    bounded_form(value)
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
    fn line_types_round_trip_through_fixed_tokens() {
        for (value, token) in [
            (LineType::Continuous, "continuous"),
            (LineType::Dashed, "dashed"),
            (LineType::Dotted, "dotted"),
            (LineType::DashDot, "dashdot"),
        ] {
            assert_eq!(value.as_str(), token);
            assert_eq!(LineType::parse(token), Some(value));
        }
        assert_eq!(LineType::parse("hidden"), None);
        assert_eq!(LineType::parse(""), None);
    }

    #[test]
    fn pathological_canonical_forms_refuse_at_admission() {
        // A binary64 whose shortest round-trip form would overflow the document's byte budget
        // is refused exactly like an oversized text string, at every numeric surface. The
        // twenty-four-byte edge itself stays valid.
        let edge = 0.0000000000000000000001_f64;
        assert_eq!(edge.to_string().len(), MAX_NUMBER_TEXT_BYTES);
        assert!(
            validate_geometry(&DraftingGeometry::Line {
                a: [edge, 0.0],
                b: [10.0, 0.0],
            })
            .is_ok()
        );
        for geometry in [
            DraftingGeometry::Line {
                a: [1e-23, 0.0],
                b: [10.0, 0.0],
            },
            DraftingGeometry::Circle {
                center: [0.0, 0.0],
                radius: 1e-23,
            },
            DraftingGeometry::Arc {
                center: [0.0, 0.0],
                radius: 5.0,
                start: 1e-23,
                end: 1.25,
            },
            DraftingGeometry::Text {
                anchor: [0.0, 0.0],
                text: "t".to_owned(),
                height: 1e-23,
                rotation: 0.0,
            },
            DraftingGeometry::Symbol {
                anchor: [0.0, 0.0],
                symbol: PaperSymbol::North,
                scale: 1e-23,
            },
            DraftingGeometry::FreeDimension {
                from: [0.0, 0.0],
                to: [30.0, 0.0],
                offset: 1e-23,
                decimal_places: 0,
            },
        ] {
            assert!(validate_geometry(&geometry).is_err(), "{geometry:?}");
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

    fn text(text: &str) -> DraftingGeometry {
        DraftingGeometry::Text {
            anchor: [10.0, 20.0],
            text: text.to_owned(),
            height: 3.5,
            rotation: 0.25,
        }
    }

    fn hatch(outline: Vec<[f64; 2]>) -> DraftingGeometry {
        DraftingGeometry::Hatch {
            outline,
            pattern: HatchPattern::Diagonal,
        }
    }

    #[test]
    fn slice_two_bounds_and_degenerate_shapes_refuse() {
        assert!(validate_geometry(&text("Note")).is_ok());
        for bad in [
            text(""),
            text(&"x".repeat(MAX_TEXT_BYTES + 1)),
            text("a\u{0007}b"),
        ] {
            assert!(validate_geometry(&bad).is_err());
        }
        assert!(
            validate_geometry(&DraftingGeometry::Text {
                anchor: [0.0, 0.0],
                text: "t".to_owned(),
                height: 0.0,
                rotation: 0.0
            })
            .is_err()
        );
        assert!(
            validate_geometry(&DraftingGeometry::Text {
                anchor: [0.0, 0.0],
                text: "t".to_owned(),
                height: 3.5,
                rotation: std::f64::consts::TAU + 1.0,
            })
            .is_err()
        );
        assert!(validate_geometry(&hatch(vec![[0.0, 0.0], [4.0, 0.0]])).is_err());
        assert!(validate_geometry(&hatch(vec![[0.0, 0.0], [4.0, 0.0], [0.0, 0.0]])).is_err());
        assert!(
            validate_geometry(&hatch(vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]])).is_ok()
        );
        assert!(
            validate_geometry(&DraftingGeometry::Symbol {
                anchor: [0.0, 0.0],
                symbol: PaperSymbol::North,
                scale: 0.0
            })
            .is_err()
        );
        assert!(
            validate_geometry(&DraftingGeometry::FreeDimension {
                from: [1.0, 1.0],
                to: [1.0, 1.0],
                offset: 8.0,
                decimal_places: 2
            })
            .is_err()
        );
        assert!(
            validate_geometry(&DraftingGeometry::FreeDimension {
                from: [0.0, 0.0],
                to: [30.0, 0.0],
                offset: MAX_PAPER_MM + 1.0,
                decimal_places: 2
            })
            .is_err()
        );
        assert!(
            validate_geometry(&DraftingGeometry::FreeDimension {
                from: [0.0, 0.0],
                to: [30.0, 0.0],
                offset: 8.0,
                decimal_places: 4
            })
            .is_err()
        );
    }

    #[test]
    fn coordinate_round_trip_through_parts_is_exact() {
        let examples = [
            (line(), None),
            (
                DraftingGeometry::Circle {
                    center: [3.0, 4.0],
                    radius: 2.5,
                },
                None,
            ),
            (
                DraftingGeometry::Arc {
                    center: [3.0, 4.0],
                    radius: 2.5,
                    start: 0.0,
                    end: 1.5,
                },
                None,
            ),
            (
                DraftingGeometry::Rectangle {
                    a: [0.0, 0.0],
                    b: [5.0, 7.0],
                },
                None,
            ),
            (
                DraftingGeometry::Polyline {
                    points: vec![[0.0, 0.0], [1.0, 2.0], [4.0, -1.0]],
                },
                None,
            ),
            (text("Label \"A\""), Some("Label \"A\"")),
            (
                hatch(vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0]]),
                Some("diagonal"),
            ),
            (
                DraftingGeometry::Hatch {
                    outline: vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0]],
                    pattern: HatchPattern::Crosshatch,
                },
                Some("crosshatch"),
            ),
            (
                DraftingGeometry::Symbol {
                    anchor: [5.0, 6.0],
                    symbol: PaperSymbol::Elevation,
                    scale: 1.5,
                },
                Some("elevation"),
            ),
            (
                DraftingGeometry::FreeDimension {
                    from: [0.0, 0.0],
                    to: [30.0, 0.0],
                    offset: 8.0,
                    decimal_places: 2,
                },
                None,
            ),
        ];
        for (geometry, payload) in examples {
            let kind = geometry.kind();
            let values = geometry.clone().coordinates();
            let rebuilt = match payload {
                // An exact-pattern payload must round-trip; a wrong one refuses.
                Some(value) => DraftingGeometry::from_parts(kind, &values, Some(value)),
                None => DraftingGeometry::from_parts(kind, &values, None),
            };
            assert_eq!(
                rebuilt,
                Some(geometry),
                "kind {kind} must survive the flat wire form exactly"
            );
            // A missing payload where the kind requires one never rebuilds.
            if payload.is_some() {
                assert_eq!(DraftingGeometry::from_parts(kind, &values, None), None);
            }
        }
        // Payload and kind must agree in both directions.
        assert_eq!(
            DraftingGeometry::from_parts("line", &[0.0, 0.0, 1.0, 0.0], Some("text")),
            None
        );
        assert_eq!(
            DraftingGeometry::from_parts("hatch", &[0.0, 0.0, 4.0, 0.0, 4.0, 4.0], Some("nope")),
            None
        );
        assert_eq!(
            DraftingGeometry::from_parts("free_dimension", &[0.0, 0.0, 30.0, 0.0, 8.0, 2.5], None),
            None
        );
    }
}
