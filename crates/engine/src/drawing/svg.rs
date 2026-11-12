// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D03 deterministic SVG export of one exact published drawing version.
//!
//! Every stroke, arrowhead and numeric glyph is contributor-original vector geometry: no font,
//! image, script, URL or other external resource is embedded or referenced. Output bytes depend
//! only on the verified published inputs; all textual metadata is escaped.

use super::drafting::{DraftingGeometry, HatchPattern, LineType, PaperSymbol};
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
/// Nominal arrowhead length backwards from each dimension-line tip.
const ARROW_MM: f64 = 3.0;
/// A short dimension line shrinks its arrowheads to fit: each head takes at most this
/// share of the line, so a 2 mm measure draws two small heads instead of two full-size
/// ones crossing into an X under the label (visual audit finding 28).
const ARROW_MAX_LINE_SHARE: f64 = 0.45;
/// Extension lines keep the drafting convention's clearance: they start a gap off the
/// measured body and run an overshoot past the dimension line, so a dimension never
/// merges with the outline it measures (visual audit finding 31).
const EXTENSION_GAP_MM: f64 = 1.5;
const EXTENSION_OVERSHOOT_MM: f64 = 1.5;
/// Label distance from the dimension line to the glyph box edge.
const LABEL_GAP_MM: f64 = 1.5;
/// Most 1 mm steps a dimension label takes along its normal to clear the body's box: two
/// text heights, so the label always stays readable as its own line's label.
const LABEL_MAX_STEPS: u32 = 7;
/// Nominal ring radius of a drawn paper symbol at scale 1, in millimetres.
const SYMBOL_RING_MM: f64 = 5.0;

const SEGMENT_TOP: [[f64; 2]; 2] = [[0.0, 3.5], [2.0, 3.5]];
const SEGMENT_MIDDLE: [[f64; 2]; 2] = [[0.0, 1.75], [2.0, 1.75]];
const SEGMENT_BOTTOM: [[f64; 2]; 2] = [[0.0, 0.0], [2.0, 0.0]];
const SEGMENT_LEFT_TOP: [[f64; 2]; 2] = [[0.0, 1.75], [0.0, 3.5]];
const SEGMENT_RIGHT_TOP: [[f64; 2]; 2] = [[2.0, 1.75], [2.0, 3.5]];
const SEGMENT_LEFT_BOTTOM: [[f64; 2]; 2] = [[0.0, 0.0], [0.0, 1.75]];
const SEGMENT_RIGHT_BOTTOM: [[f64; 2]; 2] = [[2.0, 0.0], [2.0, 1.75]];
// The digit 1 is a centred stem with a top-left flag, not the right-column pair: the
// seven-segment 1 filled only the right edge of its cell, so it read as right-aligned and
// split a number in two ("36 18.0", visual audit finding 10).
const SEGMENT_FLAG_ONE: [[f64; 2]; 2] = [[0.3, 2.7], [0.65, 3.5]];
const SEGMENT_STEM_ONE: [[f64; 2]; 2] = [[1.0, 3.5], [1.0, 0.0]];

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
        '1' => Some(&[SEGMENT_FLAG_ONE, SEGMENT_STEM_ONE]),
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
        // A B is not a narrower 8: its spine is straight and its two bowls round off on the
        // right with chamfered corners, where the digit 8 keeps square corners on both sides
        // and the letter keeps none on the left. The silhouette differs at every corner of
        // the right side, so "DET B" never reads "DET 8" (visual audit finding 26).
        'B' => Some(&[
            [[0.0, 3.5], [0.0, 0.0]],
            [[0.0, 3.5], [1.3, 3.5]],
            [[1.3, 3.5], [1.8, 3.0]],
            [[1.8, 3.0], [1.8, 2.25]],
            [[1.8, 2.25], [1.3, 1.75]],
            [[0.0, 1.75], [1.3, 1.75]],
            [[1.3, 1.75], [2.0, 1.15]],
            [[2.0, 1.15], [2.0, 0.6]],
            [[2.0, 0.6], [1.4, 0.0]],
            [[1.4, 0.0], [0.0, 0.0]],
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
        // The S carries a diagonal spine and two short verticals, so it cannot read as the
        // digit 5, whose horizontal middle bar and full-length verticals it used to share
        // with only an offset (#490 finding 2). Straight strokes only, like every capital.
        'S' => Some(&[
            [[1.8, 3.5], [0.3, 3.5]],
            [[0.3, 3.5], [0.3, 2.2]],
            [[0.3, 2.2], [1.7, 1.3]],
            [[1.7, 1.3], [1.7, 0.0]],
            [[1.7, 0.0], [0.2, 0.0]],
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
        // The engineering punctuation set (visual audit finding 11): "LEVEL +3.500",
        // "MARK B-1 (2 PCS) 50%" and friends keep every mark as original strokes. Straight
        // strokes only, on the shared 2.0 x 3.5 box like every capital.
        ',' => Some(&[[[0.9, 0.7], [0.6, -0.5]]]),
        ';' => Some(&[[[1.0, 2.2], [1.0, 1.8]], [[0.9, 0.7], [0.6, -0.5]]]),
        '/' => Some(&[[[0.2, 0.0], [1.8, 3.5]]]),
        '(' => Some(&[[[1.3, 3.8], [0.6, 1.75]], [[0.6, 1.75], [1.3, -0.3]]]),
        ')' => Some(&[[[0.6, 3.8], [1.3, 1.75]], [[1.3, 1.75], [0.6, -0.3]]]),
        '+' => Some(&[[[0.2, 1.75], [1.8, 1.75]], [[1.0, 2.55], [1.0, 0.95]]]),
        '=' => Some(&[[[0.2, 2.2], [1.8, 2.2]], [[0.2, 1.3], [1.8, 1.3]]]),
        '#' => Some(&[
            [[0.7, 0.0], [0.7, 3.5]],
            [[1.3, 0.0], [1.3, 3.5]],
            [[0.1, 2.4], [1.9, 2.4]],
            [[0.1, 1.1], [1.9, 1.1]],
        ]),
        // An angular ampersand: an open upper loop and a crossed lower loop.
        '&' => Some(&[
            [[1.8, 2.4], [0.5, 3.5]],
            [[0.5, 3.5], [0.5, 2.2]],
            [[0.5, 2.2], [1.8, 0.8]],
            [[1.8, 0.8], [1.2, 0.0]],
            [[1.2, 0.0], [0.3, 0.0]],
            [[0.3, 0.0], [1.5, 1.6]],
        ]),
        '_' => Some(&[[[0.0, -0.6], [2.0, -0.6]]]),
        // Two diamonds on a slash.
        '%' => Some(&[
            [[1.8, 0.0], [0.2, 3.5]],
            [[0.55, 2.45], [1.1, 3.0]],
            [[1.1, 3.0], [0.55, 3.5]],
            [[0.55, 3.5], [0.0, 3.0]],
            [[0.0, 3.0], [0.55, 2.45]],
            [[1.45, -0.05], [2.0, 0.5]],
            [[2.0, 0.5], [1.45, 1.05]],
            [[1.45, 1.05], [0.9, 0.5]],
            [[0.9, 0.5], [1.45, -0.05]],
        ]),
        _ => None,
    }
}

/// Whether one character draws as an original stroke glyph: everything [`glyph`] names,
/// a drawn dot, or a blank space. Text admission admits only characters that draw, so an
/// admissible text keeps every mark instead of losing them silently (visual audit
/// finding 11).
pub(super) fn draws(ch: char) -> bool {
    ch == ' ' || ch == '.' || glyph(ch.to_ascii_uppercase()).is_some()
}

/// The decimal point's stroke for a dot cell starting at `x` with its baseline at `bottom`.
/// The cell is narrower than a glyph and the previous glyph's gap precedes it, so the dot
/// is centred in the free space between the previous glyph's right edge (`x - GLYPH_GAP`)
/// and the next glyph's left edge (`x + DOT_ADVANCE`): equal clearance on both sides, so
/// "3000.000" never reads "3000 .000" (visual audit finding 27). It sits just above the
/// baseline, where a decimal point belongs.
fn decimal_point(x: f64, bottom: f64) -> [[f64; 2]; 2] {
    let centre = x + (DOT_ADVANCE - GLYPH_GAP) / 2.0;
    [
        [centre - 0.12, bottom + 0.15],
        [centre + 0.12, bottom + 0.15],
    ]
}

/// Positions the label glyphs past the dimension line, along the line's own normal on
/// the side away from the anchors. `avoid` is the body's bounding box in the same paper
/// space, when one is known: the label steps further along the normal until its glyph box
/// clears it, so a label inside a body outline never coincides with the body's strokes
/// (visual audit finding 27 — the 1:20 label's top strokes landed on the beam's top edge).
pub(super) fn label_strokes(
    label: &str,
    line: [[f64; 2]; 2],
    anchor_mid: [f64; 2],
    avoid: Option<[f64; 4]>,
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
    let mut offset = LABEL_GAP_MM + extent / 2.0;
    // Step the label along its normal until the glyph box clears the body's box (with a
    // breathing margin); bounded so a pathological body box can never loop forever.
    if let Some([lo_x, lo_y, hi_x, hi_y]) = avoid {
        let margin = 0.5;
        let box_overlaps = |offset: f64| {
            let center = [
                line_mid[0] + normal[0] * offset,
                line_mid[1] + normal[1] * offset,
            ];
            let lo = [
                center[0] - width / 2.0 - margin,
                center[1] - GLYPH_HEIGHT / 2.0 - margin,
            ];
            let hi = [
                center[0] + width / 2.0 + margin,
                center[1] + GLYPH_HEIGHT / 2.0 + margin,
            ];
            lo[0] < hi_x && hi[0] > lo_x && lo[1] < hi_y && hi[1] > lo_y
        };
        // The label may step out of the body only a short way: a label stepped across a
        // long body (a vertical dimension inside a 3000 mm beam's front view, whose normal
        // runs along the beam) would land metres away from its own line and read as
        // belonging to nothing (the regression of the finding 27 fix). Within the cap it
        // tries its own side first, then the other side of the line; when neither clears,
        // it stays beside its line.
        let base = offset;
        let steps = (0..=LABEL_MAX_STEPS).map(|k| base + f64::from(k));
        offset = steps
            .clone()
            .chain(steps.map(|o| -o))
            .find(|o| !box_overlaps(*o))
            .unwrap_or(base);
    }
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
            strokes.push(decimal_point(x, bottom));
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
            strokes.push(decimal_point(x, origin[1]));
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

/// Two open-V strokes forming an arrowhead at `tip`, opening back along the unit
/// `inward`, sized for its measured line: full size on a long line, shrunk so two heads
/// on a short line can never cross each other (visual audit finding 28).
pub(super) fn arrow_strokes_sized(
    tip: [f64; 2],
    inward: [f64; 2],
    line_length_mm: f64,
) -> [[[f64; 2]; 2]; 2] {
    let arrow = ARROW_MM.min(line_length_mm * ARROW_MAX_LINE_SHARE);
    let wing_mm = arrow / 3.0;
    let side = [-inward[1], inward[0]];
    let wing = |sign: f64| {
        [
            tip[0] + arrow * inward[0] + sign * wing_mm * side[0],
            tip[1] + arrow * inward[1] + sign * wing_mm * side[1],
        ]
    };
    [[tip, wing(1.0)], [tip, wing(-1.0)]]
}

/// One extension line: from the clearance gap off the measured point to the overshoot
/// past the dimension-line tip. A pair closer than the convention's own room stays a
/// plain segment, never a reversed one.
fn extension_line(anchor: [f64; 2], tip: [f64; 2]) -> [[f64; 2]; 2] {
    let (dx, dy) = (tip[0] - anchor[0], tip[1] - anchor[1]);
    let length = dx.hypot(dy);
    let room = EXTENSION_GAP_MM + EXTENSION_OVERSHOOT_MM;
    if !length.is_finite() || length <= room {
        return [anchor, tip];
    }
    let unit = [dx / length, dy / length];
    [
        [
            anchor[0] + unit[0] * EXTENSION_GAP_MM,
            anchor[1] + unit[1] * EXTENSION_GAP_MM,
        ],
        [
            tip[0] + unit[0] * EXTENSION_OVERSHOOT_MM,
            tip[1] + unit[1] * EXTENSION_OVERSHOOT_MM,
        ],
    ]
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

/// Renders one stroke list as path data at fixed four-decimal precision. A stroke whose
/// start continues the previous stroke's end joins it into one polyline subpath, so a dash
/// pattern runs along a tessellated circle or arc instead of restarting at every chord and
/// painting the circle solid (#490 finding 1); a break in the chain starts a new `M`.
pub(super) fn path(strokes: &[[[f64; 2]; 2]]) -> String {
    let mut text = String::new();
    let mut pen: Option<String> = None;
    for [[sx, sy], [ex, ey]] in strokes {
        let start = format!("{sx:.4} {sy:.4}");
        let end = format!("{ex:.4} {ey:.4}");
        // A closing stroke that only repeats the point the run already reached (a ring's
        // last chord ends at its first point again) adds nothing to the picture and would
        // print a duplicated point.
        if pen.as_deref() == Some(start.as_str()) && start == end {
            continue;
        }
        if pen.as_deref() == Some(start.as_str()) {
            text.push_str(&format!(" L {end}"));
        } else {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&format!("M {start} L {end}"));
        }
        pen = Some(end);
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
    let extension = normalized(vec![extension_line(a, la), extension_line(b, lb)])?;
    let line = normalized(vec![[la, lb]])?;
    let length = (lb[0] - la[0]).hypot(lb[1] - la[1]);
    if !length.is_finite() || length == 0.0 {
        return Err(invalid("the dimension line is collapsed"));
    }
    let inward = [(lb[0] - la[0]) / length, (lb[1] - la[1]) / length];
    let arrows = normalized(
        arrow_strokes_sized(la, inward, length)
            .into_iter()
            .chain(arrow_strokes_sized(lb, [-inward[0], -inward[1]], length))
            .collect(),
    )?;
    let anchor_mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let avoid = {
        let [lo_x, lo_y, hi_x, hi_y] = bounds(&body);
        Some([lo_x, lo_y, hi_x, hi_y])
    };
    let label = normalized(label_strokes(label_mm, *line_paper_mm, anchor_mid, avoid))?;
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
/// the geometry was validated at admission, so rendering only maps it. Each entity
/// contributes up to two layers: its pattern strokes under the authored line type, and
/// its text glyphs under a continuous stroke — a dash pattern never cuts a letter into
/// pieces (visual audit finding 29).
pub(super) fn drafting_layers(drawing: &Drawing) -> Result<Vec<DraftingLayer>, CommandError> {
    let mut layers = Vec::new();
    for entity in drawing.visible_drafting() {
        let pattern = normalized(pattern_strokes(&entity.geometry))?;
        if !pattern.is_empty() {
            layers.push(DraftingLayer {
                id: entity.id.to_string(),
                line_type: entity.line_type,
                strokes: pattern,
            });
        }
        let glyphs = normalized(glyph_strokes(&entity.geometry))?;
        if !glyphs.is_empty() {
            layers.push(DraftingLayer {
                id: entity.id.to_string(),
                line_type: LineType::Continuous,
                strokes: glyphs,
            });
        }
    }
    Ok(layers)
}

/// One drawn object's stroke geometry: straight strokes only, exactly like every other
/// export surface. Curved sweeps become chords at one chord per millimetre of arc,
/// bounded so a huge radius cannot blow the byte cap.
fn pattern_strokes(geometry: &DraftingGeometry) -> Vec<[[f64; 2]; 2]> {
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
        DraftingGeometry::Hatch { outline, pattern } => {
            // The named fill pattern draws as original strokes: parallel hatch lines at
            // 45 degrees, crossed for crosshatch, dense for solid (visual audit
            // finding 7 — the three patterns exported as identical empty rectangles).
            let mut strokes: Vec<[[f64; 2]; 2]> = outline
                .iter()
                .zip(outline.iter().skip(1))
                .map(|(a, b)| [*a, *b])
                .chain(outline.last().zip(outline.first()).map(|(a, b)| [*a, *b]))
                .collect();
            strokes.extend(hatch_fill(outline, *pattern));
            strokes
        }
        DraftingGeometry::Symbol {
            anchor,
            symbol,
            scale,
        } => symbol_strokes(*anchor, *symbol, *scale),
        DraftingGeometry::FreeDimension { .. } => free_dimension_strokes(geometry).0,
        // Text glyphs take their own continuous layer; nothing pattern-shaped here.
        DraftingGeometry::Text { .. } => Vec::new(),
    }
}

/// One stroke group of a drawn object.
type Strokes = Vec<[[f64; 2]; 2]>;

/// One free dimension's two stroke groups: the pattern strokes (witness lines, the
/// offset line and the arrowheads, drawn under the authored line type) and the numeric
/// label's glyphs, which always draw unbroken — a dash pattern never cuts the measured
/// number into pieces (visual audit finding 29).
fn free_dimension_strokes(geometry: &DraftingGeometry) -> (Strokes, Strokes) {
    let DraftingGeometry::FreeDimension {
        from,
        to,
        offset,
        decimal_places,
    } = geometry
    else {
        return (Vec::new(), Vec::new());
    };
    let dx = to[0] - from[0];
    let dy = to[1] - from[1];
    let length = dx.hypot(dy);
    // Admission refuses equal endpoints, so this is only a bounds guard; an impossible
    // measure renders nothing instead of dividing by zero.
    if !length.is_finite() || length == 0.0 {
        return (Vec::new(), Vec::new());
    }
    let (nx, ny) = (-dy / length, dx / length);
    let line = |p: [f64; 2]| [p[0] + nx * offset, p[1] + ny * offset];
    let la = line(*from);
    let lb = line(*to);
    let line_length = (lb[0] - la[0]).hypot(lb[1] - la[1]);
    let inward = [(lb[0] - la[0]) / line_length, (lb[1] - la[1]) / line_length];
    let label = format!("{:.*}", usize::from(*decimal_places), line_length);
    let anchor_mid = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    let pattern = vec![extension_line(*from, la), extension_line(*to, lb), [la, lb]]
        .into_iter()
        .chain(arrow_strokes_sized(la, inward, line_length))
        .chain(arrow_strokes_sized(
            lb,
            [-inward[0], -inward[1]],
            line_length,
        ))
        .collect();
    let glyphs = label_strokes(&label, [la, lb], anchor_mid, None);
    (pattern, glyphs)
}

/// The glyph strokes of one drawn object: the text run and a free dimension's numeric
/// label, always drawn unbroken.
fn glyph_strokes(geometry: &DraftingGeometry) -> Vec<[[f64; 2]; 2]> {
    match geometry {
        DraftingGeometry::Text {
            anchor,
            text,
            height,
            rotation,
        } => transformed_text(text, *anchor, height / GLYPH_HEIGHT, *rotation),
        DraftingGeometry::FreeDimension { .. } => free_dimension_strokes(geometry).1,
        _ => Vec::new(),
    }
}

/// The fill lines of one hatch: an even-odd scanline sweep of the closed contour with
/// parallel lines at 45 degrees (crossed with the mirrored sweep for crosshatch, dense
/// for solid). The outline is admission-validated paper geometry, so the sweep is
/// bounded by its own extent over the fixed spacing.
fn hatch_fill(outline: &[[f64; 2]], pattern: HatchPattern) -> Vec<[[f64; 2]; 2]> {
    let spacing = match pattern {
        HatchPattern::Solid => 0.5,
        HatchPattern::Diagonal | HatchPattern::Crosshatch => 2.0,
    };
    // Unit run directions of the fill lines at 45 degrees and its mirror.
    let sweeps = [
        [
            std::f64::consts::FRAC_1_SQRT_2,
            std::f64::consts::FRAC_1_SQRT_2,
        ],
        [
            std::f64::consts::FRAC_1_SQRT_2,
            -std::f64::consts::FRAC_1_SQRT_2,
        ],
    ];
    let sweep_count = match pattern {
        HatchPattern::Crosshatch => 2,
        _ => 1,
    };
    let mut strokes = Vec::new();
    for &[dx, dy] in sweeps.iter().take(sweep_count) {
        let n = [-dy, dx];
        let along = |[x, y]: [f64; 2], u: [f64; 2]| x * u[0] + y * u[1];
        let t_lo = outline
            .iter()
            .map(|point| along(*point, n))
            .fold(f64::INFINITY, f64::min);
        let t_hi = outline
            .iter()
            .map(|point| along(*point, n))
            .fold(f64::NEG_INFINITY, f64::max);
        if !t_lo.is_finite() || !t_hi.is_finite() {
            continue;
        }
        let mut t = (t_lo / spacing).ceil() * spacing;
        let mut lines = 0;
        while t <= t_hi {
            // Even-odd scanline: crossings of the hatch line with the contour's edges,
            // sorted along the run direction and paired into segments.
            let mut crossings: Vec<f64> = Vec::new();
            for (a, b) in outline
                .iter()
                .zip(outline.iter().skip(1))
                .chain(outline.last().zip(outline.first()))
            {
                let (ta, tb) = (along(*a, n), along(*b, n));
                if (ta <= t && tb > t) || (tb <= t && ta > t) {
                    let s = (t - ta) / (tb - ta);
                    let point = [a[0] + s * (b[0] - a[0]), a[1] + s * (b[1] - a[1])];
                    crossings.push(along(point, [dx, dy]));
                }
            }
            crossings.sort_by(f64::total_cmp);
            for &[s0, s1] in crossings.as_chunks::<2>().0 {
                strokes.push([
                    [dx * s0 + n[0] * t, dy * s0 + n[1] * t],
                    [dx * s1 + n[0] * t, dy * s1 + n[1] * t],
                ]);
            }
            t += spacing;
            lines += 1;
            // ponytail: a paper-filling solid hatch stays bounded; the cap keeps the
            // sweep linear and lets the byte cap refuse honest oversized output.
            if lines > 4_096 {
                break;
            }
        }
    }
    strokes
}

/// The closed chord ring of one paper circle, validated like every export stroke.
pub(super) fn circle_strokes(
    center: [f64; 2],
    radius: f64,
) -> Result<Vec<[[f64; 2]; 2]>, CommandError> {
    let count = chord_count(radius, std::f64::consts::TAU);
    normalized(ring(
        center,
        radius,
        0.0,
        std::f64::consts::TAU,
        count,
        true,
    ))
}

/// One placed paper symbol's own strokes: each symbol carries its convention's figure
/// inside the ring, so the four never read as the same letter-in-a-ring (visual audit
/// finding 30). All figures are contributor-original straight strokes, scaled about the
/// anchor from the nominal 10 mm ring.
fn symbol_strokes(anchor: [f64; 2], symbol: PaperSymbol, scale: f64) -> Vec<[[f64; 2]; 2]> {
    let radius = SYMBOL_RING_MM * scale;
    let count = chord_count(radius, std::f64::consts::TAU);
    let mut strokes = ring(anchor, radius, 0.0, std::f64::consts::TAU, count, true);
    let place = |[x, y]: [f64; 2]| [anchor[0] + x * scale, anchor[1] + y * scale];
    match symbol {
        // North: a vertical arrow pointing up the sheet.
        PaperSymbol::North => {
            strokes.push([place([0.0, -2.8]), place([0.0, 2.8])]);
            strokes.push([place([0.0, 2.8]), place([-1.1, 1.2])]);
            strokes.push([place([0.0, 2.8]), place([1.1, 1.2])]);
        }
        // Section: a horizontal cut line whose arrowhead names the view direction.
        PaperSymbol::Section => {
            strokes.push([place([-2.8, 0.0]), place([2.8, 0.0])]);
            strokes.push([place([2.8, 0.0]), place([1.7, 0.9])]);
            strokes.push([place([2.8, 0.0]), place([1.7, -0.9])]);
        }
        // Detail: a target — a concentric inner ring about a centre point.
        PaperSymbol::Detail => {
            let inner = radius * 0.55;
            strokes.extend(ring(
                anchor,
                inner,
                0.0,
                std::f64::consts::TAU,
                chord_count(inner, std::f64::consts::TAU),
                true,
            ));
            strokes.push([place([-0.3, 0.0]), place([0.3, 0.0])]);
            strokes.push([place([0.0, -0.3]), place([0.0, 0.3])]);
        }
        // Elevation: the convention's triangle.
        PaperSymbol::Elevation => {
            strokes.push([place([-2.2, -1.6]), place([0.0, 1.8])]);
            strokes.push([place([0.0, 1.8]), place([2.2, -1.6])]);
            strokes.push([place([-2.2, -1.6]), place([2.2, -1.6])]);
        }
    }
    strokes
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
    // A stored revision with a now-undrawable name or title exports as blanks; refuse the
    // drawn surface honestly instead (visual audit finding 11).
    drawing.ensure_drawable_text()?;
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
        drawing_with_strokes(
            name,
            DraftingGeometry::Line {
                a: [10.0, 500.0],
                b: [20.0, 510.0],
            },
            LineType::Continuous,
        )
    }

    fn drawing_with_strokes(
        name: &str,
        geometry: DraftingGeometry,
        line_type: LineType,
    ) -> Drawing {
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
                geometry,
                layer,
                line_type,
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

    /// The drawn object's path data from an export: the segment after its `data-drafting`
    /// key up to the element's close.
    fn drafting_path_data(text: &str) -> String {
        text.split("data-drafting=\"01900000-0000-7000-8000-000000000006\"")
            .nth(1)
            .unwrap_or_default()
            .split(" d=\"")
            .nth(1)
            .unwrap_or_default()
            .split("\"/>")
            .next()
            .unwrap_or_default()
            .to_owned()
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
        let label = label_strokes("2500", [[0.0, 0.0], [100.0, 100.0]], [65.35, 35.3], None);
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
        assert_ne!(canonical('B'), canonical('8'), "B must differ from 8");
        // B is not a narrower 8: its bowls round off on the right with chamfered corners
        // (strokes that are neither horizontal nor vertical), while 8 draws only square
        // corners, and its left side is one straight spine (visual audit finding 26).
        // Every stroke stays inside the shared glyph box.
        let strokes = glyph('B').unwrap_or(&[]);
        let widest = strokes
            .iter()
            .flat_map(|[[ax, _], [bx, _]]| [*ax, *bx])
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(widest <= 2.0, "B stays inside its box: {widest}");
        let chamfers = |ch: char| -> usize {
            glyph(ch)
                .unwrap_or(&[])
                .iter()
                .filter(|[[ax, ay], [bx, by]]| (ax - bx).abs() > 0.05 && (ay - by).abs() > 0.05)
                .count()
        };
        assert!(chamfers('B') >= 4, "every right corner of B is chamfered");
        assert_eq!(chamfers('8'), 0, "8 keeps square corners");
        assert!(
            strokes.contains(&[[0.0, 3.5], [0.0, 0.0]]),
            "B has one straight spine"
        );
        // The shape test above passed while S still drew the digit 5 shifted: a corner
        // alignment cannot catch a same-shape glyph on a differently sized box. What makes
        // an S an S is the diagonal spine (#490 finding 2): it has a stroke that is neither
        // horizontal nor vertical, and neither 5 nor 2 — the digits it is mistaken for —
        // carries one; and its horizontal strokes never span the full box the way 5's
        // middle bar does.
        let diagonal = |ch: char| -> bool {
            glyph(ch)
                .unwrap_or(&[])
                .iter()
                .any(|[[ax, ay], [bx, by]]| (ax - bx).abs() > 0.05 && (ay - by).abs() > 0.05)
        };
        assert!(diagonal('S'), "S carries a diagonal spine");
        assert!(!diagonal('5'), "5 has no diagonal");
        assert!(!diagonal('2'), "2 has no diagonal");
    }

    #[test]
    fn the_decimal_point_sits_midway_between_its_digits() {
        // Visual audit finding 27: the point sat 0.4 mm before the next digit and 1.9 mm
        // after the previous one, so "3000.000" read "3000 .000".
        for strokes in [
            text_strokes("0.0", [0.0, 0.0]),
            label_strokes("0.0", [[0.0, -1.0], [9.0, -1.0]], [4.5, -10.0], None),
        ] {
            let xs = |s: &[[f64; 2]; 2]| [s[0][0], s[1][0]];
            let left_digit = strokes
                .iter()
                .take(6)
                .flat_map(xs)
                .fold(f64::NEG_INFINITY, f64::max);
            let dot = strokes.get(6).copied().unwrap_or_default();
            let right_digit = strokes
                .iter()
                .skip(7)
                .flat_map(xs)
                .fold(f64::INFINITY, f64::min);
            let before = dot[0][0].min(dot[1][0]) - left_digit;
            let after = right_digit - dot[0][0].max(dot[1][0]);
            assert!(before > 0.3 && after > 0.3, "{before} {after}");
            assert!((before - after).abs() < 1e-9, "{before} vs {after}");
        }
    }

    #[test]
    fn a_label_never_walks_along_a_long_body_away_from_its_line() {
        // Regression of the finding 27 fix: a vertical height dimension inside a 150 mm
        // long body box stepped its label along the body's length and landed it past the
        // far end, metres away from its own line. The label stays beside the line.
        let line = [[5.0, 0.0], [5.0, 15.0]];
        let strokes = label_strokes("300", line, [0.0, 7.5], Some([0.0, 0.0, 150.0, 15.0]));
        let nearest = strokes
            .iter()
            .flat_map(|[[ax, _], [bx, _]]| [*ax, *bx])
            .map(|x| (x - 5.0).abs())
            .fold(f64::INFINITY, f64::min);
        // It may step a short way, here onto the other side of the line just past the body's
        // near edge, but never along the body: within the step cap of its line.
        assert!(
            nearest < 12.0,
            "the label sits beside its line: {nearest} mm away"
        );
        let farthest = strokes
            .iter()
            .flat_map(|[[ax, _], [bx, _]]| [*ax, *bx])
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(
            farthest < 20.0,
            "the label never runs along the body: {farthest}"
        );
    }

    #[test]
    fn a_label_blocked_on_its_side_takes_the_other_side_of_the_line() {
        // A body deeper than the step cap on the label's own side used to put the label back
        // on the body's strokes; the other side of the line is clear, so it goes there.
        let line = [[0.0, 0.0], [30.0, 0.0]];
        // Anchors below the line: the label's own side is above, where the body sits 0..40.
        let strokes = label_strokes("300", line, [15.0, -5.0], Some([0.0, 0.0, 30.0, 40.0]));
        let highest = strokes
            .iter()
            .flat_map(|[[_, ay], [_, by]]| [*ay, *by])
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(
            highest < 0.0,
            "the label sits below the line, off the body: {highest}"
        );
        assert!(highest > -6.0, "and still beside its line: {highest}");
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
        // The ampersand is admissible (it draws as strokes) and still exercises the XML
        // escape: an undrawable name like a<b> refuses at admission now.
        let drawing = drawing("a&b");
        let version = DocumentVersionId::parse(C).unwrap();
        let bytes = export(&drawing, version, &projection("10.5")).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.contains("a&amp;b"));
        assert!(!text.contains("a&b"));
        // "10.5" draws digit, point and digit glyphs as 2+6+1+5 original stroke subpaths,
        // never font text; the digit segments touch but never follow each other, so none
        // of them chain.
        let label = text
            .split("<path class=\"label\" d=\"")
            .nth(1)
            .unwrap_or_default();
        assert_eq!(label.matches("M ").count(), 14);
    }

    #[test]
    fn connected_strokes_chain_into_one_polyline_subpath() {
        // The rectangle's four sides run into each other, so they draw as one `M` with
        // three continuing `L`, while a disconnected pair keeps its own `M` each.
        let chained = path(&[
            [[1.0, 1.0], [5.0, 1.0]],
            [[5.0, 1.0], [5.0, 4.0]],
            [[5.0, 4.0], [1.0, 4.0]],
            [[1.0, 4.0], [1.0, 1.0]],
        ]);
        assert_eq!(chained.matches("M ").count(), 1, "{chained}");
        assert_eq!(chained.matches(" L ").count(), 4, "{chained}");
        let broken = path(&[[[1.0, 1.0], [5.0, 1.0]], [[2.0, 2.0], [6.0, 2.0]]]);
        assert_eq!(broken.matches("M ").count(), 2, "{broken}");
    }

    #[test]
    fn a_dashed_circle_draws_one_subpath_so_its_pattern_runs() {
        // #490 finding 1: every ~1 mm chord of the tessellated ring started its own
        // subpath, and SVG restarts the dash pattern per subpath, so dashed and dash-dot
        // circles exported solid and a dotted circle showed fine dashes. The chords now
        // join one polyline, and the whole dashed circle carries a single `M`.
        let drawing = drawing_with_strokes(
            "Ring",
            DraftingGeometry::Circle {
                center: [45.0, 160.0],
                radius: 12.0,
            },
            LineType::Dashed,
        );
        let version = DocumentVersionId::parse(C).unwrap();
        let bytes = export(&drawing, version, &projection("2500")).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.contains("stroke-dasharray=\"3 3\""), "{text}");
        let data = drafting_path_data(text);
        assert_eq!(
            data.matches("M ").count(),
            1,
            "the ring is one polyline subpath: {data}"
        );
        // The tessellation stays: a 12 mm radius keeps at least its 16-chord floor, and
        // the closing stroke chains into the run.
        assert!(data.matches(" L ").count() >= 16, "{data}");
    }

    #[test]
    fn an_arc_chains_its_chords_and_a_rect_keeps_one_run() {
        let drawing = drawing_with_strokes(
            "Arc",
            DraftingGeometry::Arc {
                center: [30.0, 160.0],
                radius: 12.0,
                start: 0.0,
                end: 1.5,
            },
            LineType::DashDot,
        );
        let version = DocumentVersionId::parse(C).unwrap();
        let bytes = export(&drawing, version, &projection("2500")).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(
            text.contains("stroke-dasharray=\"3 1.5 0.5 1.5\""),
            "{text}"
        );
        let data = drafting_path_data(text);
        assert_eq!(
            data.matches("M ").count(),
            1,
            "the arc's chords are one run: {data}"
        );
        let drawing = drawing_with_strokes(
            "Rectangle",
            DraftingGeometry::Rectangle {
                a: [30.0, 95.0],
                b: [60.0, 125.0],
            },
            LineType::Dashed,
        );
        let bytes = export(&drawing, version, &projection("2500")).unwrap();
        let data = drafting_path_data(std::str::from_utf8(&bytes).unwrap());
        assert_eq!(
            data.matches("M ").count(),
            1,
            "the four sides run into each other: {data}"
        );
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

    #[test]
    fn the_engineering_punctuation_set_draws_as_strokes() {
        // Visual audit finding 11: admissible text kept only capitals, digits, dash and
        // colon; everything else silently blanked. The full set draws now, and the
        // admission gate matches the drawable set exactly.
        for ch in [
            ',', ';', '/', '(', ')', '+', '=', '#', '&', '_', '%', '-', ':', 'A', '7', '.',
        ] {
            assert!(draws(ch), "{ch} draws");
        }
        assert!(draws(' '), "a space admits and advances blank");
        assert!(!draws('ä'), "non-ASCII has no stroke glyph");
    }

    #[test]
    fn extension_lines_keep_the_convention_gap_and_overshoot() {
        // Visual audit finding 31: extension lines ran from the body's own corners into
        // the dimension line, so a column read as a three-section rectangle. Each
        // extension line now starts a gap off the measured point and runs an overshoot
        // past the dimension line.
        let projected = projection("1000");
        let DimensionProjection::Resolved(dimension) = &projected.dimension else {
            panic!("resolved fixture");
        };
        let [a, b] = dimension.anchors_paper_mm;
        let [la, lb] = dimension.line_paper_mm;
        let strokes = normalized(vec![extension_line(a, la), extension_line(b, lb)]).unwrap();
        for [start, end] in &strokes {
            // Both fixture extension lines run from the anchors at y 0.05 to the line at
            // y 10.05, so up is the tip direction.
            assert!(start[1] >= 0.05 + 1.5 - 1e-9, "gap off the body: {start:?}");
            assert!(
                end[1] >= 10.05 + 1.5 - 1e-9,
                "overshoot past the line: {end:?}"
            );
        }
    }

    #[test]
    fn a_label_inside_the_body_outline_steps_out_of_it() {
        // Visual audit finding 27: at 1:20 with offset 10 the dimension line sat inside
        // the body and the label's top strokes landed on the beam's top edge. Given the
        // body's box the label steps along its normal until the glyph box clears it.
        let body_box = [0.0, 0.0, 150.0, 15.0];
        let label = label_strokes(
            "3000",
            [[10.0, 10.0], [140.0, 10.0]],
            [75.0, 0.0],
            Some(body_box),
        );
        let mut low_y = f64::INFINITY;
        for [[_, ay], [_, by]] in &label {
            low_y = low_y.min(*ay).min(*by);
        }
        assert!(
            low_y >= 15.0,
            "every glyph sits past the body's top edge: {low_y}"
        );
        // Without a body to avoid, the placement keeps its base rule.
        let plain = label_strokes("3000", [[10.0, 10.0], [140.0, 10.0]], [75.0, 0.0], None);
        let mut plain_low = f64::INFINITY;
        for [[_, ay], [_, by]] in &plain {
            plain_low = plain_low.min(*ay).min(*by);
        }
        assert!(plain_low < 15.0, "the base placement clears the line only");
    }

    #[test]
    fn a_two_millimetre_measure_draws_small_heads_not_a_cross() {
        // Visual audit finding 28: a 2 mm free dimension drew two full-size 3 mm
        // arrowheads that crossed into an X under the label. A short line shrinks its
        // heads, so the two never reach past each other's tip.
        let left = arrow_strokes_sized([0.0, 0.0], [1.0, 0.0], 2.0);
        let right = arrow_strokes_sized([2.0, 0.0], [-1.0, 0.0], 2.0);
        let span = |head: &[[[f64; 2]; 2]]| {
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            for [[ax, _], [bx, _]] in head {
                lo = lo.min(*ax).min(*bx);
                hi = hi.max(*ax).max(*bx);
            }
            [lo, hi]
        };
        let [_left_lo, left_hi] = span(&left);
        let [right_lo, _right_hi] = span(&right);
        assert!(
            left_hi <= right_lo,
            "the heads do not cross: left ends {left_hi}, right starts {right_lo}"
        );
        // A long line keeps the full-size head exactly as before.
        let long = arrow_strokes_sized([0.0, 0.0], [1.0, 0.0], 100.0);
        let [long_lo, long_hi] = span(&long);
        assert!(
            (long_hi - long_lo - 3.0).abs() < 1e-9,
            "{long_lo}..{long_hi}"
        );
    }

    #[test]
    fn the_three_hatch_patterns_draw_distinct_fills() {
        // Visual audit finding 7: solid, diagonal and crosshatch exported as three
        // identical empty rectangles. Each pattern now fills its outline with its own
        // line density and directions, all inside the outline's box.
        let outline = vec![[10.0, 10.0], [60.0, 10.0], [60.0, 40.0], [10.0, 40.0]];
        let fills = [
            HatchPattern::Solid,
            HatchPattern::Diagonal,
            HatchPattern::Crosshatch,
        ]
        .map(|pattern| hatch_fill(&outline, pattern));
        assert_ne!(fills[0], fills[1]);
        assert_ne!(fills[1], fills[2]);
        assert_ne!(fills[0], fills[2]);
        // Solid is densest, crosshatch draws two directions and so outcounts diagonal.
        assert!(fills[0].len() > fills[1].len(), "solid is dense");
        assert!(fills[2].len() > fills[1].len(), "crosshatch adds a sweep");
        let mut inside = true;
        for stroke in &fills[2] {
            for point in stroke {
                let [x, y] = *point;
                // The reconstruction carries binary64 dust around the outline's edges.
                inside &= (9.9999..=60.0001).contains(&x) && (9.9999..=40.0001).contains(&y);
            }
        }
        assert!(inside, "the fill stays inside the outline");
    }

    #[test]
    fn the_four_paper_symbols_draw_figures_of_their_own() {
        // Visual audit finding 30: north, section, detail and elevation all drew the same
        // letter-in-a-ring. Each symbol now carries its convention's figure, and the four
        // stroke sets differ.
        let drawn = [
            PaperSymbol::North,
            PaperSymbol::Section,
            PaperSymbol::Detail,
            PaperSymbol::Elevation,
        ]
        .map(|symbol| symbol_strokes([50.0, 50.0], symbol, 1.0));
        for i in 0..drawn.len() {
            for j in (i + 1)..drawn.len() {
                assert_ne!(drawn[i], drawn[j], "symbols {i} and {j} differ");
            }
            assert!(drawn[i].len() > 16, "symbol {i} keeps its ring");
        }
    }

    #[test]
    fn a_dashed_text_draws_continuous_while_a_dashed_ring_keeps_its_pattern() {
        // Visual audit finding 29: the authored dash pattern cut glyph strokes into
        // pieces ("DASHED TEXT" fell apart). Text draws on its own continuous layer;
        // a dashed circle keeps the authored pattern on its pattern layer.
        let drawing = drawing_with_strokes(
            "Dashed text",
            DraftingGeometry::Text {
                anchor: [30.0, 160.0],
                text: "DASHED TEXT".to_owned(),
                height: 3.5,
                rotation: 0.0,
            },
            LineType::Dashed,
        );
        let layers = drafting_layers(&drawing).unwrap();
        assert_eq!(layers.len(), 1, "a text entity draws one layer");
        assert_eq!(layers[0].line_type, LineType::Continuous);
        assert!(dash_array(layers[0].line_type).is_none());
        let drawing = drawing_with_strokes(
            "Dashed ring",
            DraftingGeometry::Circle {
                center: [45.0, 160.0],
                radius: 12.0,
            },
            LineType::Dashed,
        );
        let layers = drafting_layers(&drawing).unwrap();
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].line_type, LineType::Dashed);
        assert_eq!(dash_array(layers[0].line_type), Some(&[3.0, 3.0][..]));
        // The free dimension's numeric label is glyphs too: it draws unbroken on its own
        // continuous layer while its witnesses keep the authored dash (review of this PR).
        let drawing = drawing_with_strokes(
            "Dashed measure",
            DraftingGeometry::FreeDimension {
                from: [10.0, 10.0],
                to: [50.0, 10.0],
                offset: 8.0,
                decimal_places: 1,
            },
            LineType::Dashed,
        );
        let layers = drafting_layers(&drawing).unwrap();
        assert_eq!(layers.len(), 2, "pattern and glyph layers");
        assert_eq!(layers[0].line_type, LineType::Dashed);
        assert_eq!(layers[1].line_type, LineType::Continuous);
        // The glyph layer carries only the label's strokes, above the offset line.
        assert!(
            layers[1].strokes.iter().all(|stroke| stroke
                .iter()
                .all(|[x, y]| { (20.0..=40.0).contains(x) && (19.0..=27.0).contains(y) })),
            "the glyph layer holds only label strokes"
        );
    }
}
