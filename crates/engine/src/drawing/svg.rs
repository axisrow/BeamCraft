// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D03 deterministic SVG export of one exact published drawing version.
//!
//! Every stroke, arrowhead and numeric glyph is contributor-original vector geometry: no font,
//! image, script, URL or other external resource is embedded or referenced. Output bytes depend
//! only on the verified published inputs; all textual metadata is escaped.

use super::drafting::{DraftingGeometry, LineType, PaperSymbol};
use super::{Drawing, MAX_PAPER_MM};
use crate::drawing::projection::{
    ALGORITHM, BodyProjection, DimensionGeometry, DimensionProjection, Projection,
};
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::DocumentVersionId;

/// Version of the closed SVG export contract.
pub const EXPORT_CONTRACT: &str = "beamcraft-drawing-export/1";
/// Largest exported SVG, far above any bounded first-slice drawing.
pub const MAX_EXPORT_BYTES: usize = 65_536;

/// Paper margin around every stroke in the viewport.
pub(super) const MARGIN_MM: f64 = 5.0;
/// Stroke glyph digit box: 2.0 mm wide, 3.5 mm high, 1.0 mm advance gap.
const GLYPH_WIDTH: f64 = 2.0;
const GLYPH_HEIGHT: f64 = 3.5;
const GLYPH_GAP: f64 = 1.0;
const DOT_ADVANCE: f64 = 1.5;
/// Arrowhead length backwards from each dimension-line tip.
const ARROW_MM: f64 = 3.0;
/// Label distance from the dimension line to the glyph box edge.
const LABEL_GAP_MM: f64 = 1.5;
/// Nominal ring radius of a drawn paper symbol at scale 1, in millimetres.
const SYMBOL_RING_MM: f64 = 5.0;

const SEGMENT_TOP: [[f64; 2]; 2] = [[0.0, 3.5], [2.0, 3.5]];
const SEGMENT_MIDDLE: [[f64; 2]; 2] = [[0.0, 1.75], [2.0, 1.75]];
const SEGMENT_BOTTOM: [[f64; 2]; 2] = [[0.0, 0.0], [2.0, 0.0]];
const SEGMENT_LEFT_TOP: [[f64; 2]; 2] = [[0.0, 1.75], [0.0, 3.5]];
const SEGMENT_RIGHT_TOP: [[f64; 2]; 2] = [[2.0, 1.75], [2.0, 3.5]];
const SEGMENT_LEFT_BOTTOM: [[f64; 2]; 2] = [[0.0, 0.0], [0.0, 1.75]];
const SEGMENT_RIGHT_BOTTOM: [[f64; 2]; 2] = [[2.0, 0.0], [2.0, 1.75]];

fn invalid(message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::InvalidArgument, "drawing_export", message)
}

/// Escapes text for XML character content, covering every markup-significant character.
pub(super) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Original stroke glyphs for the bounded millimetre label; an unknown character advances a
/// blank gap instead of a fabricated glyph.
pub(super) fn glyph(digit: char) -> Option<&'static [[[f64; 2]; 2]]> {
    match digit {
        '0' => Some(&[
            SEGMENT_TOP,
            SEGMENT_LEFT_TOP,
            SEGMENT_RIGHT_TOP,
            SEGMENT_LEFT_BOTTOM,
            SEGMENT_RIGHT_BOTTOM,
            SEGMENT_BOTTOM,
        ]),
        '1' => Some(&[SEGMENT_RIGHT_TOP, SEGMENT_RIGHT_BOTTOM]),
        '2' => Some(&[
            SEGMENT_TOP,
            SEGMENT_RIGHT_TOP,
            SEGMENT_MIDDLE,
            SEGMENT_LEFT_BOTTOM,
            SEGMENT_BOTTOM,
        ]),
        '3' => Some(&[
            SEGMENT_TOP,
            SEGMENT_RIGHT_TOP,
            SEGMENT_MIDDLE,
            SEGMENT_RIGHT_BOTTOM,
            SEGMENT_BOTTOM,
        ]),
        '4' => Some(&[
            SEGMENT_LEFT_TOP,
            SEGMENT_MIDDLE,
            SEGMENT_RIGHT_TOP,
            SEGMENT_RIGHT_BOTTOM,
        ]),
        '5' => Some(&[
            SEGMENT_TOP,
            SEGMENT_LEFT_TOP,
            SEGMENT_MIDDLE,
            SEGMENT_RIGHT_BOTTOM,
            SEGMENT_BOTTOM,
        ]),
        '6' => Some(&[
            SEGMENT_TOP,
            SEGMENT_LEFT_TOP,
            SEGMENT_MIDDLE,
            SEGMENT_LEFT_BOTTOM,
            SEGMENT_RIGHT_BOTTOM,
            SEGMENT_BOTTOM,
        ]),
        '7' => Some(&[SEGMENT_TOP, SEGMENT_RIGHT_TOP, SEGMENT_RIGHT_BOTTOM]),
        '8' => Some(&[
            SEGMENT_TOP,
            SEGMENT_LEFT_TOP,
            SEGMENT_RIGHT_TOP,
            SEGMENT_MIDDLE,
            SEGMENT_LEFT_BOTTOM,
            SEGMENT_RIGHT_BOTTOM,
            SEGMENT_BOTTOM,
        ]),
        '9' => Some(&[
            SEGMENT_TOP,
            SEGMENT_LEFT_TOP,
            SEGMENT_RIGHT_TOP,
            SEGMENT_MIDDLE,
            SEGMENT_RIGHT_BOTTOM,
            SEGMENT_BOTTOM,
        ]),
        // Contributor-original angular capitals on the same 2.0 × 3.5 box as the digits:
        // straight strokes only, so a title block renders without any font resource.
        'A' => Some(&[
            [[0.0, 0.0], [1.0, 3.5]],
            [[1.0, 3.5], [2.0, 0.0]],
            [[0.4, 1.2], [1.6, 1.2]],
        ]),
        'B' => Some(&[
            [[0.0, 3.5], [0.0, 0.0]],
            [[0.0, 3.5], [1.4, 3.5]],
            [[1.4, 3.5], [1.4, 1.75]],
            [[0.0, 1.75], [1.4, 1.75]],
            [[0.0, 0.0], [1.1, 0.0]],
            [[1.1, 0.0], [1.1, 1.75]],
        ]),
        'C' => Some(&[
            [[2.0, 3.5], [0.0, 3.5]],
            [[0.0, 3.5], [0.0, 0.0]],
            [[0.0, 0.0], [2.0, 0.0]],
        ]),
        'D' => Some(&[
            [[0.0, 3.5], [0.0, 0.0]],
            [[0.0, 3.5], [1.6, 3.5]],
            [[1.6, 3.5], [2.0, 2.5]],
            [[2.0, 2.5], [2.0, 1.0]],
            [[2.0, 1.0], [1.6, 0.0]],
            [[1.6, 0.0], [0.0, 0.0]],
        ]),
        'E' => Some(&[
            [[2.0, 3.5], [0.0, 3.5]],
            [[0.0, 3.5], [0.0, 0.0]],
            [[0.0, 0.0], [2.0, 0.0]],
            [[0.0, 1.75], [1.5, 1.75]],
        ]),
        'F' => Some(&[
            [[2.0, 3.5], [0.0, 3.5]],
            [[0.0, 3.5], [0.0, 0.0]],
            [[0.0, 1.75], [1.5, 1.75]],
        ]),
        'G' => Some(&[
            [[2.0, 3.5], [0.0, 3.5]],
            [[0.0, 3.5], [0.0, 0.0]],
            [[0.0, 0.0], [2.0, 0.0]],
            [[2.0, 0.0], [2.0, 1.2]],
            [[2.0, 1.2], [1.2, 1.2]],
        ]),
        'H' => Some(&[
            [[0.0, 3.5], [0.0, 0.0]],
            [[2.0, 3.5], [2.0, 0.0]],
            [[0.0, 1.75], [2.0, 1.75]],
        ]),
        'I' => Some(&[
            [[1.0, 3.5], [1.0, 0.0]],
            [[0.4, 3.5], [1.6, 3.5]],
            [[0.4, 0.0], [1.6, 0.0]],
        ]),
        'J' => Some(&[[[1.8, 3.5], [1.8, 0.0]], [[1.8, 0.0], [0.2, 0.0]]]),
        'K' => Some(&[
            [[0.0, 3.5], [0.0, 0.0]],
            [[0.0, 1.9], [2.0, 3.5]],
            [[0.0, 1.9], [2.0, 0.0]],
        ]),
        'L' => Some(&[[[0.0, 3.5], [0.0, 0.0]], [[0.0, 0.0], [2.0, 0.0]]]),
        'M' => Some(&[
            [[0.0, 0.0], [0.0, 3.5]],
            [[0.0, 3.5], [1.0, 1.75]],
            [[1.0, 1.75], [2.0, 3.5]],
            [[2.0, 3.5], [2.0, 0.0]],
        ]),
        'N' => Some(&[
            [[0.0, 0.0], [0.0, 3.5]],
            [[0.0, 3.5], [2.0, 0.0]],
            [[2.0, 0.0], [2.0, 3.5]],
        ]),
        'O' => Some(&[
            [[0.6, 3.5], [1.4, 3.5]],
            [[1.4, 3.5], [2.0, 2.9]],
            [[2.0, 2.9], [2.0, 0.6]],
            [[2.0, 0.6], [1.4, 0.0]],
            [[1.4, 0.0], [0.6, 0.0]],
            [[0.6, 0.0], [0.0, 0.6]],
            [[0.0, 0.6], [0.0, 2.9]],
            [[0.0, 2.9], [0.6, 3.5]],
        ]),
        'P' => Some(&[
            [[0.0, 3.5], [0.0, 0.0]],
            [[0.0, 3.5], [1.8, 3.5]],
            [[1.8, 3.5], [1.8, 1.75]],
            [[1.8, 1.75], [0.0, 1.75]],
        ]),
        'Q' => Some(&[
            [[0.0, 3.5], [2.0, 3.5]],
            [[2.0, 3.5], [2.0, 0.0]],
            [[2.0, 0.0], [0.0, 0.0]],
            [[0.0, 0.0], [0.0, 3.5]],
            [[1.2, 1.2], [2.0, 0.0]],
        ]),
        'R' => Some(&[
            [[0.0, 3.5], [0.0, 0.0]],
            [[0.0, 3.5], [1.8, 3.5]],
            [[1.8, 3.5], [1.8, 1.75]],
            [[1.8, 1.75], [0.0, 1.75]],
            [[1.0, 1.75], [2.0, 0.0]],
        ]),
        'S' => Some(&[
            [[2.0, 3.5], [0.4, 3.5]],
            [[0.4, 3.5], [0.4, 1.75]],
            [[0.4, 1.75], [1.6, 1.75]],
            [[1.6, 1.75], [1.6, 0.0]],
            [[1.6, 0.0], [0.0, 0.0]],
        ]),
        'T' => Some(&[[[0.0, 3.5], [2.0, 3.5]], [[1.0, 3.5], [1.0, 0.0]]]),
        'U' => Some(&[
            [[0.0, 3.5], [0.0, 0.0]],
            [[0.0, 0.0], [2.0, 0.0]],
            [[2.0, 0.0], [2.0, 3.5]],
        ]),
        'V' => Some(&[[[0.0, 3.5], [1.0, 0.0]], [[1.0, 0.0], [2.0, 3.5]]]),
        'W' => Some(&[
            [[0.0, 3.5], [0.5, 0.0]],
            [[0.5, 0.0], [1.0, 2.0]],
            [[1.0, 2.0], [1.5, 0.0]],
            [[1.5, 0.0], [2.0, 3.5]],
        ]),
        'X' => Some(&[[[0.0, 3.5], [2.0, 0.0]], [[0.0, 0.0], [2.0, 3.5]]]),
        'Y' => Some(&[
            [[0.0, 3.5], [1.0, 1.75]],
            [[2.0, 3.5], [1.0, 1.75]],
            [[1.0, 1.75], [1.0, 0.0]],
        ]),
        'Z' => Some(&[
            [[0.0, 3.5], [2.0, 3.5]],
            [[2.0, 3.5], [0.0, 0.0]],
            [[0.0, 0.0], [2.0, 0.0]],
        ]),
        ':' => Some(&[[[1.0, 2.4], [1.0, 2.0]], [[1.0, 1.5], [1.0, 1.1]]]),
        '-' => Some(&[[[0.3, 1.75], [1.7, 1.75]]]),
        _ => None,
    }
}

/// Positions the label glyphs past the dimension line, along the line's own normal on
/// the side away from the anchors.
pub(super) fn label_strokes(
    label: &str,
    line: [[f64; 2]; 2],
    anchor_mid: [f64; 2],
) -> Vec<[[f64; 2]; 2]> {
    let [la, lb] = line;
    let line_mid = [(la[0] + lb[0]) / 2.0, (la[1] + lb[1]) / 2.0];
    let width: f64 = label
        .chars()
        .map(|ch| {
            if ch == '.' {
                DOT_ADVANCE
            } else {
                GLYPH_WIDTH + GLYPH_GAP
            }
        })
        .sum::<f64>()
        - GLYPH_GAP;
    // The label sits at the label gap past the line, along the line's own normal, on
    // the side away from the anchors, and the box is offset by half its extent along
    // that normal — so every line direction, the horizontal main frame and the 45
    // degree knee brace alike, keeps its glyphs clear of the strokes (visual audit
    // finding 4). Glyphs keep their own upright direction on every side.
    let (dx, dy) = (lb[0] - la[0], lb[1] - la[1]);
    let length = dx.hypot(dy);
    let toward = [anchor_mid[0] - line_mid[0], anchor_mid[1] - line_mid[1]];
    let normal = if length > 0.0 {
        let mut n = [-dy / length, dx / length];
        if n[0] * toward[0] + n[1] * toward[1] > 0.0 {
            n = [-n[0], -n[1]];
        }
        n
    } else if line_mid[1] >= anchor_mid[1] {
        [0.0, 1.0]
    } else {
        [0.0, -1.0]
    };
    let extent = width * normal[0].abs() + GLYPH_HEIGHT * normal[1].abs();
    let offset = LABEL_GAP_MM + extent / 2.0;
    let mut x = line_mid[0] + normal[0] * offset - width / 2.0;
    let bottom = line_mid[1] + normal[1] * offset - GLYPH_HEIGHT / 2.0;
    let mut strokes = Vec::new();
    for ch in label.chars() {
        let advance = if ch == '.' {
            DOT_ADVANCE
        } else {
            GLYPH_WIDTH + GLYPH_GAP
        };
        if ch == '.' {
            strokes.push([[x + 0.9, bottom], [x + 1.1, bottom]]);
        } else if let Some(segments) = glyph(ch) {
            for [[ax, ay], [bx, by]] in segments {
                strokes.push([[x + ax, bottom + ay], [x + bx, bottom + by]]);
            }
        }
        x += advance;
    }
    strokes
}

/// Left-aligned glyph strokes of one bounded text run with its bottom-left corner at
/// `origin`. ASCII lowercase folds onto its capital and a `.` draws its dot, so admissible
/// title-block text keeps every mark; a character without any glyph advances a blank gap
/// instead of a fabricated mark, changing only the spacing, never the drawn strokes.
pub(super) fn text_strokes(text: &str, origin: [f64; 2]) -> Vec<[[f64; 2]; 2]> {
    let mut strokes = Vec::new();
    let mut x = origin[0];
    for ch in text.chars() {
        let folded = ch.to_ascii_uppercase();
        let advance = if folded == '.' {
            DOT_ADVANCE
        } else {
            GLYPH_WIDTH + GLYPH_GAP
        };
        if folded == '.' {
            strokes.push([[x + 0.9, origin[1]], [x + 1.1, origin[1]]]);
        } else if let Some(segments) = glyph(folded) {
            for [[ax, ay], [bx, by]] in segments {
                strokes.push([[x + ax, origin[1] + ay], [x + bx, origin[1] + by]]);
            }
        }
        x += advance;
    }
    strokes
}

/// Total advance width of one text run exactly as [`text_strokes`] lays it out, so callers
/// can centre a run without duplicating the advance rule. Never negative.
pub(super) fn text_width(text: &str) -> f64 {
    let advances: f64 = text
        .chars()
        .map(|ch| {
            let folded = ch.to_ascii_uppercase();
            if folded == '.' {
                DOT_ADVANCE
            } else {
                GLYPH_WIDTH + GLYPH_GAP
            }
        })
        .sum();
    (advances - GLYPH_GAP).max(0.0)
}

/// Two open-V strokes forming an arrowhead at `tip`, opening back along the unit `inward`.
pub(super) fn arrow_strokes(tip: [f64; 2], inward: [f64; 2]) -> [[[f64; 2]; 2]; 2] {
    let side = [-inward[1], inward[0]];
    let wing = |sign: f64| {
        [
            tip[0] + ARROW_MM * inward[0] + sign * side[0],
            tip[1] + ARROW_MM * inward[1] + sign * side[1],
        ]
    };
    [[tip, wing(1.0)], [tip, wing(-1.0)]]
}

/// Validates and normalizes one paper point; negative zero normalizes so bytes stay canonical.
pub(super) fn checked(point: [f64; 2]) -> Result<[f64; 2], CommandError> {
    let [x, y] = point;
    if !x.is_finite() || !y.is_finite() {
        return Err(CommandError::new(
            ErrorCode::NonFiniteNumber,
            "drawing_export",
            "export received a nonfinite coordinate",
        ));
    }
    if x.abs() > MAX_PAPER_MM || y.abs() > MAX_PAPER_MM {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "drawing_export",
            "export received a coordinate beyond the paper cap",
        ));
    }
    Ok([
        if x == 0.0 { 0.0 } else { x },
        if y == 0.0 { 0.0 } else { y },
    ])
}

/// Validates a stroke list and returns the normalized copy used for geometry and bytes.
pub(super) fn normalized(strokes: Vec<[[f64; 2]; 2]>) -> Result<Vec<[[f64; 2]; 2]>, CommandError> {
    strokes
        .into_iter()
        .map(|[start, end]| Ok([checked(start)?, checked(end)?]))
        .collect()
}

/// Renders one stroke list as absolute-move/line subpath data at fixed four-decimal precision.
pub(super) fn path(strokes: &[[[f64; 2]; 2]]) -> String {
    let mut text = String::new();
    for [[sx, sy], [ex, ey]] in strokes {
        text.push_str(&format!(
            "M {sx:.4} {sy:.4} L {ex:.4} {ey:.4} ",
            sx = sx,
            sy = sy,
            ex = ex,
            ey = ey
        ));
    }
    text
}

/// The five published stroke groups in draw order: body, extension, dimension line,
/// arrowheads and label glyphs. Every coordinate is validated and normalized.
pub(super) type StrokeGroups = [Vec<[[f64; 2]; 2]>; 5];

/// Resolves and validates the exact published geometry once for every export surface
/// (SVG, PDF, print). Unresolved geometry, an empty body and unbounded coordinates refuse;
/// returns the groups and the model-unit dimension label used by export metadata.
pub(super) fn stroke_groups(
    projection: &Projection,
) -> Result<(StrokeGroups, String), CommandError> {
    let (BodyProjection::Resolved(edges), DimensionProjection::Resolved(dimension)) =
        (&projection.body, &projection.dimension)
    else {
        return Err(invalid("unresolved drawing geometry cannot be exported"));
    };
    if edges.is_empty() {
        return Err(invalid("a published drawing has at least one stroke"));
    }
    let DimensionGeometry {
        anchors_paper_mm,
        line_paper_mm,
        label_mm,
        ..
    } = dimension;
    let [a, b] = *anchors_paper_mm;
    let [la, lb] = *line_paper_mm;
    let body = normalized(edges.iter().map(|edge| edge.paper_mm).collect())?;
    let extension = normalized(vec![[a, la], [b, lb]])?;
    let line = normalized(vec![[la, lb]])?;
    let length = (lb[0] - la[0]).hypot(lb[1] - la[1]);
    if !length.is_finite() || length == 0.0 {
        return Err(invalid("the dimension line is collapsed"));
    }
    let inward = [(lb[0] - la[0]) / length, (lb[1] - la[1]) / length];
    let arrows = normalized(
        arrow_strokes(la, inward)
            .into_iter()
            .chain(arrow_strokes(lb, [-inward[0], -inward[1]]))
            .collect(),
    )?;
    let anchor_mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let label = normalized(label_strokes(label_mm, *line_paper_mm, anchor_mid))?;
    Ok((
        [body, extension, line, arrows, label],
        (*label_mm).to_owned(),
    ))
}

/// Tight bounds of validated strokes: `[min_x, min_y, max_x, max_y]` in paper mm.
pub(super) fn bounds<'a>(strokes: impl IntoIterator<Item = &'a [[f64; 2]; 2]>) -> [f64; 4] {
    let mut low = [f64::INFINITY; 2];
    let mut high = [f64::NEG_INFINITY; 2];
    for [start, end] in strokes {
        for [x, y] in [*start, *end] {
            low[0] = low[0].min(x);
            low[1] = low[1].min(y);
            high[0] = high[0].max(x);
            high[1] = high[1].max(y);
        }
    }
    [low[0], low[1], high[0], high[1]]
}

/// Maps paper strokes (Y-up) into SVG's Y-down space about `top`, the paper y shown at
/// the viewport's top edge: loose paper flips about zero, a sheet about its height. Paper
/// is Y-up and SVG renders the first view-box unit downward, so an unflipped export
/// mirrors the whole sheet (visual audit finding 1).
pub(super) fn flipped_about(strokes: Vec<[[f64; 2]; 2]>, top: f64) -> Vec<[[f64; 2]; 2]> {
    strokes
        .into_iter()
        .map(|[[ax, ay], [bx, by]]| [[ax, top - ay], [bx, top - by]])
        .collect()
}

/// One rendered drawn object: identity, authored line type and its paper-millimetre
/// strokes in document order.
pub(super) struct DraftingLayer {
    /// The drawn object's id, the SVG `data-drafting` key.
    pub id: String,
    /// The authored stroke pattern.
    pub line_type: LineType,
    /// Paper-millimetre strokes of the object's geometry.
    pub strokes: Vec<[[f64; 2]; 2]>,
}

/// The dash pattern of one line type in millimetres; continuous stays unbroken.
pub(super) fn dash_array(line_type: LineType) -> Option<&'static [f64]> {
    match line_type {
        LineType::Continuous => None,
        LineType::Dashed => Some(&[3.0, 3.0]),
        LineType::Dotted => Some(&[0.5, 1.5]),
        LineType::DashDot => Some(&[3.0, 1.5, 0.5, 1.5]),
    }
}

/// Renders every drawn object on a visible layer as paper-millimetre strokes, in
/// document order. Visibility is the paper-only filter of [`Drawing::visible_drafting`];
/// the geometry was validated at admission, so rendering only maps it.
pub(super) fn drafting_layers(drawing: &Drawing) -> Result<Vec<DraftingLayer>, CommandError> {
    let mut layers = Vec::new();
    for entity in drawing.visible_drafting() {
        layers.push(DraftingLayer {
            id: entity.id.to_string(),
            line_type: entity.line_type,
            strokes: normalized(geometry_strokes(&entity.geometry))?,
        });
    }
    Ok(layers)
}

/// One drawn object's stroke geometry: straight strokes only, exactly like every other
/// export surface. Curved sweeps become chords at one chord per millimetre of arc,
/// bounded so a huge radius cannot blow the byte cap.
fn geometry_strokes(geometry: &DraftingGeometry) -> Vec<[[f64; 2]; 2]> {
    match geometry {
        DraftingGeometry::Line { a, b } => vec![[*a, *b]],
        DraftingGeometry::Polyline { points } => points
            .iter()
            .zip(points.iter().skip(1))
            .map(|(a, b)| [*a, *b])
            .collect(),
        DraftingGeometry::Arc {
            center,
            radius,
            start,
            end,
        } => {
            let sweep = (*end - *start).rem_euclid(std::f64::consts::TAU);
            let count = chord_count(*radius, sweep);
            ring(*center, *radius, *start, sweep, count, false)
        }
        DraftingGeometry::Circle { center, radius } => {
            let count = chord_count(*radius, std::f64::consts::TAU);
            ring(*center, *radius, 0.0, std::f64::consts::TAU, count, true)
        }
        DraftingGeometry::Rectangle { a, b } => {
            let [ax, ay] = *a;
            let [bx, by] = *b;
            vec![
                [*a, [bx, ay]],
                [[bx, ay], *b],
                [*b, [ax, by]],
                [[ax, by], *a],
            ]
        }
        DraftingGeometry::Text {
            anchor,
            text,
            height,
            rotation,
        } => transformed_text(text, *anchor, height / GLYPH_HEIGHT, *rotation),
        DraftingGeometry::Hatch { outline, .. } => {
            // ponytail: the named fill pattern renders later; the closed outline already
            // places the hatch on the paper and its pattern never carried semantics.
            outline
                .iter()
                .zip(outline.iter().skip(1))
                .map(|(a, b)| [*a, *b])
                .chain(outline.last().zip(outline.first()).map(|(a, b)| [*a, *b]))
                .collect()
        }
        DraftingGeometry::Symbol {
            anchor,
            symbol,
            scale,
        } => {
            // Contributor-original marker: a ring scaled from its nominal 10 mm
            // diameter, with the symbol's initial centred inside it.
            let radius = SYMBOL_RING_MM * scale;
            let count = chord_count(radius, std::f64::consts::TAU);
            let mut strokes = ring(*anchor, radius, 0.0, std::f64::consts::TAU, count, true);
            let letter = match symbol {
                PaperSymbol::North => "N",
                PaperSymbol::Section => "S",
                PaperSymbol::Detail => "D",
                PaperSymbol::Elevation => "E",
            };
            strokes.extend(transformed_text(
                letter,
                [
                    anchor[0] - text_width(letter) * scale / 2.0,
                    anchor[1] - GLYPH_HEIGHT * scale / 2.0,
                ],
                *scale,
                0.0,
            ));
            strokes
        }
        DraftingGeometry::FreeDimension {
            from,
            to,
            offset,
            decimal_places,
        } => {
            let dx = to[0] - from[0];
            let dy = to[1] - from[1];
            let length = dx.hypot(dy);
            // Admission refuses equal endpoints, so this is only a bounds guard; an
            // impossible measure renders nothing instead of dividing by zero.
            if !length.is_finite() || length == 0.0 {
                return Vec::new();
            }
            let (nx, ny) = (-dy / length, dx / length);
            let line = |p: [f64; 2]| [p[0] + nx * offset, p[1] + ny * offset];
            let la = line(*from);
            let lb = line(*to);
            let line_length = (lb[0] - la[0]).hypot(lb[1] - la[1]);
            let inward = [(lb[0] - la[0]) / line_length, (lb[1] - la[1]) / line_length];
            let label = format!("{:.*}", usize::from(*decimal_places), length);
            let anchor_mid = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
            vec![[*from, la], [*to, lb], [la, lb]]
                .into_iter()
                .chain(arrow_strokes(la, inward))
                .chain(arrow_strokes(lb, [-inward[0], -inward[1]]))
                .chain(label_strokes(&label, [la, lb], anchor_mid))
                .collect()
        }
    }
}

/// The chord count of a circular sweep: one chord per millimetre of arc, clamped so a
/// tiny sweep keeps at least a lens and a huge radius stays inside the byte cap.
fn chord_count(radius: f64, sweep: f64) -> usize {
    ((radius * sweep).ceil() as usize).clamp(16, 256)
}

/// Straight chords along a circular arc from `start` counter-clockwise over `sweep`.
fn arc_chords(
    center: [f64; 2],
    radius: f64,
    start: f64,
    sweep: f64,
    count: usize,
) -> Vec<[f64; 2]> {
    (0..=count)
        .map(|step| {
            let angle = start + sweep * step as f64 / count as f64;
            [
                center[0] + radius * angle.cos(),
                center[1] + radius * angle.sin(),
            ]
        })
        .collect()
}

/// Strokes along a circular sweep; `close` joins the last point back to the first.
fn ring(
    center: [f64; 2],
    radius: f64,
    start: f64,
    sweep: f64,
    count: usize,
    close: bool,
) -> Vec<[[f64; 2]; 2]> {
    let points = arc_chords(center, radius, start, sweep, count);
    let mut strokes: Vec<[[f64; 2]; 2]> = points
        .iter()
        .zip(points.iter().skip(1))
        .map(|(a, b)| [*a, *b])
        .collect();
    if close && let (Some(&first), Some(&last)) = (points.first(), points.last()) {
        strokes.push([last, first]);
    }
    strokes
}

/// Glyph strokes of `text` scaled from the unit glyph box, rotated counter-clockwise by
/// `rotation` about the anchor. The box stays contributor-original geometry at every size.
fn transformed_text(text: &str, anchor: [f64; 2], scale: f64, rotation: f64) -> Vec<[[f64; 2]; 2]> {
    let (sin, cos) = rotation.sin_cos();
    let place = |[x, y]: [f64; 2]| {
        [
            anchor[0] + (x * cos - y * sin) * scale,
            anchor[1] + (x * sin + y * cos) * scale,
        ]
    };
    text_strokes(text, [0.0, 0.0])
        .into_iter()
        .map(|[a, b]| [place(a), place(b)])
        .collect()
}

/// One `<path>` element line: the class, the optional `data-view`/`data-drafting` key,
/// the authored dash pattern and the stroke data flipped into Y-down space about `top`.
pub(super) fn path_element(
    class: &str,
    key: Option<(&'static str, &str)>,
    dash: Option<&[f64]>,
    strokes: &[[[f64; 2]; 2]],
    top: f64,
) -> String {
    let flipped = flipped_about(strokes.to_vec(), top);
    let data = key
        .map(|(name, id)| format!(" {}=\"{}\"", name, escape(id)))
        .unwrap_or_default();
    let pattern = dash
        .map(|dash| {
            format!(
                " stroke-dasharray=\"{}\"",
                dash.iter()
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        })
        .unwrap_or_default();
    format!(
        "<path class=\"{class}\"{data}{pattern} d=\"{}\"/>\n",
        path(&flipped),
    )
}

/// Renders the exact resolved published geometry as deterministic stroke-only SVG bytes.
/// Unresolved geometry, unbounded coordinates and an over-cap result refuse; every drawn
/// object on a visible layer renders on the paper; the caller's model and document are
/// never touched.
pub fn export(
    drawing: &Drawing,
    version: DocumentVersionId,
    projection: &Projection,
) -> Result<Vec<u8>, CommandError> {
    let (groups, label_mm) = stroke_groups(projection)?;
    let drafting = drafting_layers(drawing)?;
    let [body, extension, line, arrows, label] = groups;
    let body = flipped_about(body, 0.0);
    let extension = flipped_about(extension, 0.0);
    let line = flipped_about(line, 0.0);
    let arrows = flipped_about(arrows, 0.0);
    let label = flipped_about(label, 0.0);
    // Bounds live in the painted (flipped) space too: a drawn object outside the view's
    // paper extent must widen the viewBox instead of painting outside it and being
    // clipped from the export.
    let flipped_drafting: Vec<Vec<[[f64; 2]; 2]>> = drafting
        .iter()
        .map(|layer| flipped_about(layer.strokes.clone(), 0.0))
        .collect();
    let [min_x, min_y, max_x, max_y] = bounds(
        [&body, &extension, &line, &arrows, &label]
            .into_iter()
            .flatten()
            .chain(flipped_drafting.iter().flatten()),
    );
    let lo_x = ((min_x - MARGIN_MM) * 100.0).floor() / 100.0;
    let lo_y = ((min_y - MARGIN_MM) * 100.0).floor() / 100.0;
    let width = ((max_x + MARGIN_MM) * 100.0).ceil() / 100.0 - lo_x;
    let height = ((max_y + MARGIN_MM) * 100.0).ceil() / 100.0 - lo_y;
    let spec = drawing.spec();
    let metadata = format!(
        "{EXPORT_CONTRACT} drawing={} view={} dimension={} version={} source_revision={} \
         source_manifest={} algorithm={} scale_denominator={} label_mm={}",
        escape(&spec.id.to_string()),
        escape(&spec.view.id.to_string()),
        escape(&spec.dimension.id.to_string()),
        escape(&version.to_string()),
        escape(&spec.source.revision.to_string()),
        escape(&spec.source.manifest.to_string()),
        ALGORITHM,
        spec.view.scale_denominator,
        escape(&label_mm),
    );
    let mut text = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <svg xmlns=\"http://www.w3.org/2000/svg\" version=\"1.1\" width=\"{width:.2}mm\" \
         height=\"{height:.2}mm\" viewBox=\"{lo_x:.2} {lo_y:.2} {width:.2} {height:.2}\">\n\
         <title>{}</title>\n<metadata>{metadata}</metadata>\n\
         <g fill=\"none\" stroke=\"#000000\" stroke-width=\"0.25\" stroke-linecap=\"round\">\n\
         <path class=\"body\" d=\"{}\"/>\n<path class=\"extension\" d=\"{}\"/>\n\
         <path class=\"dimension\" d=\"{}\"/>\n<path class=\"arrow\" d=\"{}\"/>\n\
         <path class=\"label\" d=\"{}\"/>\n",
        escape(&spec.name),
        path(&body),
        path(&extension),
        path(&line),
        path(&arrows),
        path(&label),
    );
    for layer in &drafting {
        text.push_str(&path_element(
            "drafting",
            Some(("data-drafting", &layer.id)),
            dash_array(layer.line_type),
            &layer.strokes,
            0.0,
        ));
    }
    text.push_str("</g>\n</svg>\n");
    if text.len() > MAX_EXPORT_BYTES {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "drawing_export",
            "the exported drawing exceeds its byte cap",
        ));
    }
    Ok(text.into_bytes())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::drawing::projection::{ProjectedEdge, UnresolvedReason};
    use crate::drawing::{
        DEFAULT_SCALE_DENOMINATOR, DimensionSpec, DraftingGeometry, DraftingSpec, DrawingSpec,
        LayerSpec, LineType, Plane, SourceModelPin, ViewSpec,
    };
    use crate::query::feature::FeatureId;
    use beamcraft_model::digest::ContentDigest;
    use beamcraft_model::id::{
        DimensionId, DraftingEntityId, DrawingId, DrawingViewId, ElementId, ProjectId, RevisionId,
    };

    const A: &str = "01900000-0000-7000-8000-000000000001";
    const B: &str = "01900000-0000-7000-8000-000000000002";
    const C: &str = "01900000-0000-7000-8000-000000000003";
    const MEMBER: &str = "01900000-0000-7000-8000-000000000004";

    fn drawing(name: &str) -> Drawing {
        Drawing::admit(DrawingSpec {
            id: DrawingId::parse(A).unwrap(),
            name: name.to_owned(),
            source: SourceModelPin {
                project: ProjectId::parse(A).unwrap(),
                revision: RevisionId::parse(B).unwrap(),
                manifest: ContentDigest::from_bytes([1; 32]),
                schema: ContentDigest::from_bytes([2; 32]),
                generation: 42,
            },
            member: ElementId::parse(MEMBER).unwrap(),
            view: ViewSpec {
                id: DrawingViewId::parse(B).unwrap(),
                plane: Plane::Front,
                scale_denominator: DEFAULT_SCALE_DENOMINATOR,
                paper_origin_mm: [0.0, 0.0],
                title: None,
            },
            dimension: DimensionSpec {
                id: DimensionId::parse(C).unwrap(),
                anchors: ["v.x0.y0.z0", "v.x0.y0.z1"]
                    .map(|role| FeatureId::parse(&format!("fid1:{MEMBER}:member:{role}")).unwrap()),
                paper_offset_mm: 10.0,
                decimal_places: 0,
            },
            drafting: Vec::new(),
            layers: Vec::new(),
            views: Vec::new(),
            sections: Vec::new(),
            details: Vec::new(),
            sheet: None,
        })
        .unwrap()
    }

    fn projection(label: &str) -> Projection {
        Projection {
            body: BodyProjection::Resolved(vec![ProjectedEdge {
                feature: FeatureId::parse(&format!("fid1:{MEMBER}:member:e.x0.y0")).unwrap(),
                paper_mm: [[0.1, 0.05], [10.1, 0.05]],
            }]),
            dimension: DimensionProjection::Resolved(DimensionGeometry {
                anchors_paper_mm: [[0.1, 0.05], [10.1, 0.05]],
                line_paper_mm: [[0.1, 10.05], [10.1, 10.05]],
                projected_length_mm: 1000.0,
                label_mm: label.to_owned(),
            }),
        }
    }

    #[test]
    fn bytes_are_deterministic_structured_and_self_contained() {
        let drawing = drawing("Member projection");
        let version = DocumentVersionId::parse(C).unwrap();
        let first = export(&drawing, version, &projection("1000")).unwrap();
        let second = export(&drawing, version, &projection("1000")).unwrap();
        assert_eq!(first, second);
        assert!(first.len() <= MAX_EXPORT_BYTES);
        let text = std::str::from_utf8(&first).unwrap();
        assert!(text.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(text.ends_with("</svg>\n"));
        assert!(text.contains(&format!("version={version}")));
        assert!(text.contains("label_mm=1000"));
        assert!(text.contains("M 0.1000 -0.0500 L 10.1000 -0.0500"));
        for hostile in ["<script", "<image", "<text", "href", "url(", "@import"] {
            assert!(!text.contains(hostile));
        }
        // A different label changes the glyphs, so the bytes are content-bound.
        assert_ne!(
            export(&drawing, version, &projection("995")).unwrap(),
            first
        );
    }

    #[test]
    fn exported_geometry_renders_upright_in_svg_space() {
        // Paper is Y-up while SVG renders Y-down, so the unflipped writer mirrored the
        // drawing: a 2500 mm column's label read 5200 (visual audit finding 1). The
        // topmost paper stroke must keep the smallest SVG y of the two horizontal edges.
        let drawing = drawing("Member projection");
        let version = DocumentVersionId::parse(C).unwrap();
        let mut projected = projection("2500");
        if let BodyProjection::Resolved(edges) = &mut projected.body {
            edges.push(ProjectedEdge {
                feature: FeatureId::parse(&format!("fid1:{MEMBER}:member:e.x0.z1")).unwrap(),
                paper_mm: [[0.1, 125.05], [10.1, 125.05]],
            });
        }
        let bytes = export(&drawing, version, &projected).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        let body = text
            .split("<path class=\"body\" d=\"")
            .nth(1)
            .unwrap_or_default();
        let body = body
            .split("<path class=\"extension\"")
            .next()
            .unwrap_or_default();
        // The top edge (paper y 125.05) flips above the bottom edge (paper y 0.05).
        assert!(
            body.contains("M 0.1000 -125.0500 L 10.1000 -125.0500"),
            "{body}"
        );
        assert!(
            body.contains("M 0.1000 -0.0500 L 10.1000 -0.0500"),
            "{body}"
        );
    }

    #[test]
    fn a_loose_export_bounds_every_flipped_drawn_object() {
        // Regression: the viewBox mixed flipped dimension groups with unflipped drafted
        // strokes, so a drawn object beyond the view's paper extent painted outside the
        // viewport and was clipped from the export. The drafted line reaches paper y 500,
        // so the painted bounds must reach flipped y -510, never 500.
        let drawing = drawing_with_drafting("Member projection");
        let version = DocumentVersionId::parse(C).unwrap();
        let bytes = export(&drawing, version, &projection("2500")).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        let view_box = text
            .split("viewBox=\"")
            .nth(1)
            .unwrap_or_default()
            .split('"')
            .next()
            .unwrap_or_default();
        assert!(view_box.contains("-515.00"), "{view_box}");
        assert!(!view_box.contains("495.00"), "{view_box}");
    }

    fn drawing_with_drafting(name: &str) -> Drawing {
        let layer = DraftingEntityId::parse("01900000-0000-7000-8000-000000000005").unwrap();
        Drawing::admit(DrawingSpec {
            id: DrawingId::parse(A).unwrap(),
            name: name.to_owned(),
            source: SourceModelPin {
                project: ProjectId::parse(A).unwrap(),
                revision: RevisionId::parse(B).unwrap(),
                manifest: ContentDigest::from_bytes([1; 32]),
                schema: ContentDigest::from_bytes([2; 32]),
                generation: 42,
            },
            member: ElementId::parse(MEMBER).unwrap(),
            view: ViewSpec {
                id: DrawingViewId::parse(B).unwrap(),
                plane: Plane::Front,
                scale_denominator: DEFAULT_SCALE_DENOMINATOR,
                paper_origin_mm: [0.0, 0.0],
                title: None,
            },
            dimension: DimensionSpec {
                id: DimensionId::parse(C).unwrap(),
                anchors: ["v.x0.y0.z0", "v.x0.y0.z1"]
                    .map(|role| FeatureId::parse(&format!("fid1:{MEMBER}:member:{role}")).unwrap()),
                paper_offset_mm: 10.0,
                decimal_places: 0,
            },
            drafting: vec![DraftingSpec {
                id: DraftingEntityId::parse("01900000-0000-7000-8000-000000000006").unwrap(),
                geometry: DraftingGeometry::Line {
                    a: [10.0, 500.0],
                    b: [20.0, 510.0],
                },
                layer,
                line_type: LineType::Continuous,
            }],
            layers: vec![LayerSpec {
                id: layer,
                name: "Notes".to_owned(),
                visible: true,
            }],
            views: Vec::new(),
            sections: Vec::new(),
            details: Vec::new(),
            sheet: None,
        })
        .unwrap()
    }

    #[test]
    fn a_vertical_dimension_label_sits_beside_the_line() {
        // Finding 4: the label always centred on the line's midpoint and its offset rule
        // only handled a horizontal line, so a vertical dimension line ran through its
        // own label ("25|00"). Every glyph must clear the line on the side away from the
        // anchors.
        let drawing = drawing("Member projection");
        let version = DocumentVersionId::parse(C).unwrap();
        let mut projected = projection("2500");
        if let DimensionProjection::Resolved(geometry) = &mut projected.dimension {
            geometry.anchors_paper_mm = [[10.1, 0.05], [10.1, 125.05]];
            geometry.line_paper_mm = [[0.1, 0.05], [0.1, 125.05]];
        }
        let bytes = export(&drawing, version, &projected).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        let label = text
            .split("<path class=\"label\" d=\"")
            .nth(1)
            .unwrap_or_default();
        let label = label.split("\"/>").next().unwrap_or_default();
        // Path data alternates x and y; the line runs at paper x 0.1, so the label's
        // rightmost glyph point must stay at least the 1.5 mm label gap left of it.
        let numbers: Vec<f64> = label
            .split_whitespace()
            .filter_map(|token| token.parse::<f64>().ok())
            .collect();
        let highest_x = numbers
            .iter()
            .enumerate()
            .filter(|(index, _)| index % 2 == 0)
            .fold(f64::NEG_INFINITY, |max, (_, value)| max.max(*value));
        assert!(highest_x <= 0.1 - 1.5, "{label}");
    }

    #[test]
    fn an_inclined_dimension_label_clears_its_line() {
        // The inclined half of finding 4: the steep-line rule left a 45-degree
        // knee-brace dimension on the above/below path, where the shallow line crosses
        // the glyph box of a wide label. Every glyph point must sit at least the label
        // gap away from the line, measured across it.
        let label = label_strokes("2500", [[0.0, 0.0], [100.0, 100.0]], [65.35, 35.3]);
        for stroke in &label {
            for point in stroke {
                let distance = (point[1] - point[0]).abs() / std::f64::consts::SQRT_2;
                assert!(
                    distance + 1e-6 >= 1.5,
                    "{point:?} sits {distance} from the line"
                );
            }
        }
    }

    #[test]
    fn letters_b_s_and_o_have_shapes_of_their_own() {
        // Finding 7: S shared 5's strokes exactly, O shared 0's, and B differed from 8
        // only by a 0.2 mm narrower box, so a mark B1 was indistinguishable from 81.
        // Every letter draws a shape of its own on the shared glyph box.
        let canonical = |ch: char| -> Vec<[[f64; 2]; 2]> {
            let segments = glyph(ch).unwrap_or(&[]);
            let min_x = segments
                .iter()
                .flat_map(|[[ax, _], [bx, _]]| [*ax, *bx])
                .fold(f64::INFINITY, f64::min);
            let min_y = segments
                .iter()
                .flat_map(|[[_, ay], [_, by]]| [*ay, *by])
                .fold(f64::INFINITY, f64::min);
            segments
                .iter()
                .map(|[[ax, ay], [bx, by]]| [[ax - min_x, ay - min_y], [bx - min_x, by - min_y]])
                .collect()
        };
        // Shifted to a common corner the letter and the digit it resembled must not
        // become the same shape.
        assert_ne!(canonical('S'), canonical('5'), "S must differ from 5");
        assert_ne!(canonical('O'), canonical('0'), "O must differ from 0");
        // B draws visibly inside the box: its right-side strokes sit well left of the
        // full-width digits, not merely 0.2 mm inside.
        let right_edge = glyph('B')
            .unwrap_or(&[])
            .iter()
            .flat_map(|[[ax, _], [bx, _]]| [*ax, *bx])
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(right_edge <= 1.4, "B's right side at {right_edge}");
    }

    #[test]
    fn the_colon_draws_two_dots_not_one() {
        // Finding 9: the colon was one short stroke, so every scale read "1·20". A colon
        // is two dots: two strokes whose vertical bands are disjoint and straddle the
        // glyph box's midline, inside the box the digits use.
        let strokes = glyph(':').unwrap_or(&[]);
        assert_eq!(strokes.len(), 2, "a colon is two dots, got {strokes:?}");
        let mut bands: Vec<[f64; 2]> = strokes
            .iter()
            .map(
                |[[_, ay], [_, by]]| {
                    if ay <= by { [*ay, *by] } else { [*by, *ay] }
                },
            )
            .collect();
        bands.sort_by(|a, b| a[0].total_cmp(&b[0]));
        assert!(
            bands[1][0] > bands[0][1],
            "the dots must not touch: {bands:?}"
        );
        assert!(
            bands[0][0] > 0.0 && bands[1][1] < 3.5,
            "both dots stay inside the glyph box: {bands:?}"
        );
    }

    #[test]
    fn unresolved_geometry_and_out_of_range_coordinates_refuse() {
        let drawing = drawing("Member projection");
        let version = DocumentVersionId::parse(C).unwrap();
        let unresolved = Projection {
            body: BodyProjection::Unresolved(UnresolvedReason::MissingOwner),
            dimension: DimensionProjection::Unresolved(UnresolvedReason::MissingOwner),
        };
        assert_eq!(
            export(&drawing, version, &unresolved).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        let mut nonfinite = projection("1000");
        if let BodyProjection::Resolved(edges) = &mut nonfinite.body
            && let Some(edge) = edges.first_mut()
        {
            let [[x, _], _] = &mut edge.paper_mm;
            *x = f64::NAN;
        }
        assert_eq!(
            export(&drawing, version, &nonfinite).unwrap_err().code,
            ErrorCode::NonFiniteNumber
        );
        let mut unbounded = projection("1000");
        if let BodyProjection::Resolved(edges) = &mut unbounded.body
            && let Some(edge) = edges.first_mut()
        {
            let [[_, _], [x, _]] = &mut edge.paper_mm;
            *x = MAX_PAPER_MM * 2.0;
        }
        assert_eq!(
            export(&drawing, version, &unbounded).unwrap_err().code,
            ErrorCode::LimitExceeded
        );
    }

    #[test]
    fn textual_metadata_is_escaped_and_label_glyphs_are_original_strokes() {
        let drawing = drawing("a<b>&\"c'");
        let version = DocumentVersionId::parse(C).unwrap();
        let bytes = export(&drawing, version, &projection("10.5")).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.contains("a&lt;b&gt;&amp;&quot;c&#39;"));
        assert!(!text.contains("a<b>"));
        // "10.5" draws digit, point and digit glyphs as 2+6+1+5 original stroke subpaths,
        // never font text.
        let label = text
            .split("<path class=\"label\" d=\"")
            .nth(1)
            .unwrap_or_default();
        assert_eq!(label.matches("M ").count(), 14);
    }

    #[test]
    fn title_text_folds_ascii_case_and_draws_the_dot_instead_of_blanking() {
        // Admissible mixed-case text with a dot keeps every mark: "st. mary" folds onto its
        // capitals and draws the same strokes as its uppercase form.
        assert_eq!(
            text_strokes("st. mary", [0.0, 0.0]),
            text_strokes("ST. MARY", [0.0, 0.0])
        );
        // Five capitals, the dot and nothing for the space.
        assert_eq!(text_strokes("st. mary", [0.0, 0.0]).len(), 23);
        // A space or a character without a glyph still advances blank, never fabricated.
        assert!(text_strokes("    ", [0.0, 0.0]).is_empty());
    }
}
