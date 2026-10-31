// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D03 deterministic SVG export of one exact published drawing version.
//!
//! Every stroke, arrowhead and numeric glyph is contributor-original vector geometry: no font,
//! image, script, URL or other external resource is embedded or referenced. Output bytes depend
//! only on the verified published inputs; all textual metadata is escaped.

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
            [[0.0, 3.5], [1.8, 3.5]],
            [[1.8, 3.5], [1.8, 1.75]],
            [[0.0, 1.75], [1.8, 1.75]],
            [[1.8, 1.75], [1.8, 0.0]],
            [[0.0, 0.0], [1.8, 0.0]],
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
            [[0.0, 3.5], [2.0, 3.5]],
            [[2.0, 3.5], [2.0, 0.0]],
            [[2.0, 0.0], [0.0, 0.0]],
            [[0.0, 0.0], [0.0, 3.5]],
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
            [[2.0, 3.5], [0.0, 3.5]],
            [[0.0, 3.5], [0.0, 1.75]],
            [[0.0, 1.75], [2.0, 1.75]],
            [[2.0, 1.75], [2.0, 0.0]],
            [[2.0, 0.0], [0.0, 0.0]],
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
        ':' => Some(&[[[1.0, 2.4], [1.0, 2.0]]]),
        '-' => Some(&[[[0.3, 1.75], [1.7, 1.75]]]),
        _ => None,
    }
}

/// Positions the label glyphs centred on the dimension line, on the side the line is offset to.
pub(super) fn label_strokes(
    label: &str,
    line_mid: [f64; 2],
    anchor_mid: [f64; 2],
) -> Vec<[[f64; 2]; 2]> {
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
    // Above the line when the line sits above the anchors, else below; glyphs keep their own
    // upright direction on both sides.
    let above = line_mid[1] >= anchor_mid[1];
    let bottom = if above {
        line_mid[1] + LABEL_GAP_MM
    } else {
        line_mid[1] - LABEL_GAP_MM - GLYPH_HEIGHT
    };
    let mut strokes = Vec::new();
    let mut x = line_mid[0] - width / 2.0;
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
    let line_mid = [(la[0] + lb[0]) / 2.0, (la[1] + lb[1]) / 2.0];
    let anchor_mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let label = normalized(label_strokes(label_mm, line_mid, anchor_mid))?;
    Ok((
        [body, extension, line, arrows, label],
        (*label_mm).to_owned(),
    ))
}

/// Tight bounds of all validated strokes: `[min_x, min_y, max_x, max_y]` in paper mm.
pub(super) fn bounds(groups: &StrokeGroups) -> [f64; 4] {
    let mut low = [f64::INFINITY; 2];
    let mut high = [f64::NEG_INFINITY; 2];
    for group in groups {
        for [start, end] in group {
            for [x, y] in [*start, *end] {
                low[0] = low[0].min(x);
                low[1] = low[1].min(y);
                high[0] = high[0].max(x);
                high[1] = high[1].max(y);
            }
        }
    }
    [low[0], low[1], high[0], high[1]]
}

/// Renders the exact resolved published geometry as deterministic stroke-only SVG bytes.
/// Unresolved geometry, unbounded coordinates and an over-cap result refuse; the caller's
/// model and document are never touched.
pub fn export(
    drawing: &Drawing,
    version: DocumentVersionId,
    projection: &Projection,
) -> Result<Vec<u8>, CommandError> {
    let (groups, label_mm) = stroke_groups(projection)?;
    let [min_x, min_y, max_x, max_y] = bounds(&groups);
    let lo_x = ((min_x - MARGIN_MM) * 100.0).floor() / 100.0;
    let lo_y = ((min_y - MARGIN_MM) * 100.0).floor() / 100.0;
    let width = ((max_x + MARGIN_MM) * 100.0).ceil() / 100.0 - lo_x;
    let height = ((max_y + MARGIN_MM) * 100.0).ceil() / 100.0 - lo_y;
    let [body, extension, line, arrows, label] = groups;
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
    let text = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <svg xmlns=\"http://www.w3.org/2000/svg\" version=\"1.1\" width=\"{width:.2}mm\" \
         height=\"{height:.2}mm\" viewBox=\"{lo_x:.2} {lo_y:.2} {width:.2} {height:.2}\">\n\
         <title>{}</title>\n<metadata>{metadata}</metadata>\n\
         <g fill=\"none\" stroke=\"#000000\" stroke-width=\"0.25\" stroke-linecap=\"round\">\n\
         <path class=\"body\" d=\"{}\"/>\n<path class=\"extension\" d=\"{}\"/>\n\
         <path class=\"dimension\" d=\"{}\"/>\n<path class=\"arrow\" d=\"{}\"/>\n\
         <path class=\"label\" d=\"{}\"/>\n</g>\n</svg>\n",
        escape(&spec.name),
        path(&body),
        path(&extension),
        path(&line),
        path(&arrows),
        path(&label),
    );
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
        DEFAULT_SCALE_DENOMINATOR, DimensionSpec, DrawingSpec, Plane, SourceModelPin, ViewSpec,
    };
    use crate::query::feature::FeatureId;
    use beamcraft_model::digest::ContentDigest;
    use beamcraft_model::id::{
        DimensionId, DrawingId, DrawingViewId, ElementId, ProjectId, RevisionId,
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
        assert!(text.contains("M 0.1000 0.0500 L 10.1000 0.0500"));
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
