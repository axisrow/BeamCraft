// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Deterministic, self-contained SVG for one verified immutable published drawing revision.

use super::store::DocumentRevision;
use crate::plate::{PlateError, integrity};
use beamcraft_engine::drawing::projection::{self, BodyProjection, DimensionProjection};
use beamcraft_model::error::ErrorCode;

const CANVAS_MARGIN_MM: f64 = 5.0;
const VECTOR_TEXT_HEIGHT_MM: f64 = 3.5;
const VECTOR_TEXT_ADVANCE_MM: f64 = 2.5;
const SVG_BYTE_CAP: usize = 128 * 1024;

fn point([x, y]: [f64; 2]) -> (f64, f64) {
    (x, y)
}

fn path_line(
    out: &mut String,
    a: (f64, f64),
    b: (f64, f64),
    transform: impl Fn((f64, f64)) -> (f64, f64),
) {
    let a = transform(a);
    let b = transform(b);
    out.push_str(&format!(
        "<path d=\"M {} {} L {} {}\"/>\n",
        a.0, a.1, b.0, b.1
    ));
}

fn arrow(
    out: &mut String,
    tip: (f64, f64),
    toward: (f64, f64),
    transform: impl Fn((f64, f64)) -> (f64, f64),
) {
    let dx = toward.0 - tip.0;
    let dy = toward.1 - tip.1;
    let length = dx.hypot(dy);
    if length <= f64::EPSILON {
        return;
    }
    let ux = dx / length;
    let uy = dy / length;
    let size = 1.5;
    let half = 0.55;
    let base = (tip.0 + ux * size, tip.1 + uy * size);
    let a = transform((base.0 - uy * half, base.1 + ux * half));
    let b = transform((base.0 + uy * half, base.1 - ux * half));
    let tip = transform(tip);
    out.push_str(&format!(
        "<path d=\"M {} {} L {} {} L {} {} Z\"/>\n",
        tip.0, tip.1, a.0, a.1, b.0, b.1
    ));
}

fn glyph_segments(ch: char) -> Option<&'static [(f64, f64, f64, f64)]> {
    const TOP: (f64, f64, f64, f64) = (0.1, 0.0, 1.8, 0.0);
    const UPPER_LEFT: (f64, f64, f64, f64) = (0.0, 0.1, 0.0, 1.5);
    const UPPER_RIGHT: (f64, f64, f64, f64) = (2.0, 0.1, 2.0, 1.5);
    const MIDDLE: (f64, f64, f64, f64) = (0.1, 1.7, 1.8, 1.7);
    const LOWER_LEFT: (f64, f64, f64, f64) = (0.0, 1.9, 0.0, 3.3);
    const LOWER_RIGHT: (f64, f64, f64, f64) = (2.0, 1.9, 2.0, 3.3);
    const BOTTOM: (f64, f64, f64, f64) = (0.1, 3.4, 1.8, 3.4);
    const DIGIT_0: &[(f64, f64, f64, f64)] = &[
        TOP,
        UPPER_LEFT,
        UPPER_RIGHT,
        LOWER_LEFT,
        LOWER_RIGHT,
        BOTTOM,
    ];
    const DIGIT_1: &[(f64, f64, f64, f64)] = &[UPPER_RIGHT, LOWER_RIGHT];
    const DIGIT_2: &[(f64, f64, f64, f64)] = &[TOP, UPPER_RIGHT, MIDDLE, LOWER_LEFT, BOTTOM];
    const DIGIT_3: &[(f64, f64, f64, f64)] = &[TOP, UPPER_RIGHT, MIDDLE, LOWER_RIGHT, BOTTOM];
    const DIGIT_4: &[(f64, f64, f64, f64)] = &[UPPER_LEFT, UPPER_RIGHT, MIDDLE, LOWER_RIGHT];
    const DIGIT_5: &[(f64, f64, f64, f64)] = &[TOP, UPPER_LEFT, MIDDLE, LOWER_RIGHT, BOTTOM];
    const DIGIT_6: &[(f64, f64, f64, f64)] =
        &[TOP, UPPER_LEFT, MIDDLE, LOWER_LEFT, LOWER_RIGHT, BOTTOM];
    const DIGIT_7: &[(f64, f64, f64, f64)] = &[TOP, UPPER_RIGHT, LOWER_RIGHT];
    const DIGIT_8: &[(f64, f64, f64, f64)] = &[
        TOP,
        UPPER_LEFT,
        UPPER_RIGHT,
        MIDDLE,
        LOWER_LEFT,
        LOWER_RIGHT,
        BOTTOM,
    ];
    const DIGIT_9: &[(f64, f64, f64, f64)] =
        &[TOP, UPPER_LEFT, UPPER_RIGHT, MIDDLE, LOWER_RIGHT, BOTTOM];
    const MINUS: &[(f64, f64, f64, f64)] = &[MIDDLE];
    const DOT: &[(f64, f64, f64, f64)] = &[(1.3, 3.35, 1.35, 3.4)];
    const M: &[(f64, f64, f64, f64)] = &[
        (0.0, 3.4, 0.0, 0.0),
        (0.0, 0.0, 1.0, 1.1),
        (1.0, 1.1, 2.0, 0.0),
        (2.0, 0.0, 2.0, 3.4),
    ];
    match ch {
        '0' => Some(DIGIT_0),
        '1' => Some(DIGIT_1),
        '2' => Some(DIGIT_2),
        '3' => Some(DIGIT_3),
        '4' => Some(DIGIT_4),
        '5' => Some(DIGIT_5),
        '6' => Some(DIGIT_6),
        '7' => Some(DIGIT_7),
        '8' => Some(DIGIT_8),
        '9' => Some(DIGIT_9),
        '-' => Some(MINUS),
        '.' => Some(DOT),
        'm' => Some(M),
        _ => None,
    }
}

fn label(
    out: &mut String,
    value: &str,
    center: (f64, f64),
    scale: f64,
    transform: impl Fn((f64, f64)) -> (f64, f64),
) -> Result<(), PlateError> {
    if value.len() > 32 {
        return Err(PlateError {
            code: ErrorCode::LimitExceeded,
            message: "drawing label exceeds the SVG glyph bound",
        });
    }
    let width = value.chars().count() as f64 * VECTOR_TEXT_ADVANCE_MM * scale;
    let origin = (
        center.0 - width / 2.0,
        center.1 - VECTOR_TEXT_HEIGHT_MM * scale / 2.0,
    );
    for (index, ch) in value.chars().enumerate() {
        let segments =
            glyph_segments(ch).ok_or_else(|| integrity("drawing label has no vector glyph"))?;
        let x = origin.0 + index as f64 * VECTOR_TEXT_ADVANCE_MM * scale;
        for (x1, y1, x2, y2) in segments {
            path_line(
                out,
                (x + x1 * scale, origin.1 + y1 * scale),
                (x + x2 * scale, origin.1 + y2 * scale),
                &transform,
            );
        }
    }
    Ok(())
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Generates a canonical SVG with no fonts, image references, timestamps, or external resources.
pub fn encode(revision: &DocumentRevision) -> Result<Vec<u8>, PlateError> {
    if revision.state != super::State::Published || revision.version.is_none() {
        return Err(PlateError {
            code: ErrorCode::InvalidArgument,
            message: "SVG export requires an immutable published drawing version",
        });
    }
    let projection = projection::resolve(&revision.drawing, &revision.source.inspect()?.snapshot)?;
    let body = match projection.body {
        BodyProjection::Resolved(edges) => edges,
        BodyProjection::Unresolved(_) => {
            return Err(PlateError {
                code: ErrorCode::DependencyUnavailable,
                message: "SVG export requires resolved body geometry",
            });
        }
    };
    let dimension = match projection.dimension {
        DimensionProjection::Resolved(dimension) => dimension,
        DimensionProjection::Unresolved(_) => {
            return Err(PlateError {
                code: ErrorCode::DependencyUnavailable,
                message: "SVG export requires a resolved dimension",
            });
        }
    };
    let mut points = Vec::with_capacity(body.len() * 2 + 4);
    for edge in &body {
        points.extend(edge.paper_mm.map(point));
    }
    points.extend(dimension.anchors_paper_mm.map(point));
    points.extend(dimension.line_paper_mm.map(point));
    let min_x = points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let max_x = points.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    let min_y = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let max_y = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    if ![min_x, max_x, min_y, max_y].into_iter().all(f64::is_finite) {
        return Err(integrity("SVG geometry bounds are not finite"));
    }
    let geometry_width = (max_x - min_x).max(1.0);
    let geometry_height = (max_y - min_y).max(1.0);
    let scale = (100.0 / geometry_width.max(geometry_height)).min(1.0);
    let label_width = dimension.label_mm.len() as f64 * VECTOR_TEXT_ADVANCE_MM * scale;
    let margin_x = CANVAS_MARGIN_MM + label_width / 2.0;
    let width = geometry_width * scale + 2.0 * margin_x;
    let height = geometry_height * scale + 2.0 * CANVAS_MARGIN_MM;
    let transform = |(x, y): (f64, f64)| {
        (
            margin_x + (x - min_x) * scale,
            CANVAS_MARGIN_MM + (max_y - y) * scale,
        )
    };
    let mut svg = String::with_capacity(4096);
    svg.push_str(&format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" version=\"1.1\" width=\"{width}mm\" height=\"{height}mm\" viewBox=\"0 0 {width} {height}\" data-drawing-id=\"{}\" data-version-id=\"{}\" data-source-revision=\"{}\" data-source-manifest=\"{}\" data-dimension-mm=\"{}\">\n<g fill=\"none\" stroke=\"#111111\" stroke-width=\"0.25\" stroke-linecap=\"round\" stroke-linejoin=\"round\">\n",
        escape(&revision.drawing.spec().id.to_string()),
        escape(&revision.version.ok_or_else(|| integrity("published version id is missing"))?.to_string()),
        escape(&revision.source.pin().revision.to_string()),
        escape(&revision.source.pin().manifest.to_string()),
        dimension.projected_length_mm,
    ));
    for edge in body {
        let [a, b] = edge.paper_mm.map(point);
        let a = transform(a);
        let b = transform(b);
        svg.push_str(&format!(
            "<path data-feature-id=\"{}\" d=\"M {} {} L {} {}\"/>\n",
            escape(&edge.feature.to_string()),
            a.0,
            a.1,
            b.0,
            b.1
        ));
    }
    let [anchor_a, anchor_b] = dimension.anchors_paper_mm.map(point);
    let [line_a, line_b] = dimension.line_paper_mm.map(point);
    path_line(&mut svg, anchor_a, line_a, transform);
    path_line(&mut svg, anchor_b, line_b, transform);
    path_line(&mut svg, line_a, line_b, transform);
    arrow(&mut svg, line_a, line_b, transform);
    arrow(&mut svg, line_b, line_a, transform);
    let mid = (
        (line_a.0 + line_b.0) / 2.0,
        (line_a.1 + line_b.1) / 2.0 + 2.5,
    );
    label(&mut svg, &dimension.label_mm, mid, scale, transform)?;
    svg.push_str("</g>\n</svg>\n");
    if svg.len() > SVG_BYTE_CAP {
        return Err(PlateError {
            code: ErrorCode::LimitExceeded,
            message: "SVG drawing exceeds its output byte cap",
        });
    }
    Ok(svg.into_bytes())
}

/// Writes exact bytes to a new destination; an existing file is never truncated or replaced.
pub fn write_new(path: &std::path::Path, bytes: &[u8]) -> Result<(), PlateError> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| PlateError {
            code: ErrorCode::InvalidArgument,
            message: "SVG destination exists or cannot be created",
        })?;
    if file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .is_err()
    {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(PlateError {
            code: ErrorCode::BackendUnavailable,
            message: "SVG output could not be written",
        });
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn glyph_set_is_closed_to_numeric_dimension_labels_and_units() {
        for ch in "0123456789-.m".chars() {
            assert!(glyph_segments(ch).is_some());
        }
        assert!(glyph_segments('x').is_none());
    }
}
