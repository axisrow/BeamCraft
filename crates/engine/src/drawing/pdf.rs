// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D08: PDF output of one exact published drawing version.
//!
//! The single-page PDF 1.4 document contains only path-painting operators over the same
//! contributor-original stroke geometry as the SVG export: no font is embedded, referenced
//! or substituted, so every glyph and symbol remains BeamCraft's own vector drawing
//! (`AGENTS.md` §1). Bytes depend only on the verified published inputs — no clock, host or
//! random object identity — and the caller's model and document are never touched.

use super::Drawing;
use super::projection::{ALGORITHM, Projection};
use super::svg::{MARGIN_MM, MAX_EXPORT_BYTES, bounds, drafting_layers, stroke_groups};
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::DocumentVersionId;

/// Version of the closed PDF export contract.
pub const PDF_EXPORT_CONTRACT: &str = "beamcraft-drawing-pdf/1";

/// Millimetre to PDF point conversion at this IO edge (1 inch = 25.4 mm = 72 pt).
const MM_TO_PT: f64 = 72.0 / 25.4;
/// Stroke width matching the SVG export, in millimetres.
const STROKE_MM: f64 = 0.25;

/// One painted path of a page: its strokes and the authored dash pattern in paper
/// millimetres, when the path carries one.
pub(super) struct PagePath {
    /// Paper-millimetre strokes.
    pub strokes: Vec<[[f64; 2]; 2]>,
    /// The dash pattern in millimetres; unbroken when `None`.
    pub dash_mm: Option<&'static [f64]>,
}

/// Escapes one PDF literal string: backslash and parentheses get backslash escapes and
/// every byte outside printable ASCII becomes a three-digit octal escape, so arbitrary
/// bounded label text stays inert inside the object dict.
fn escape_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'\\' | b'(' | b')' => {
                out.push('\\');
                out.push(byte as char);
            }
            0x20..=0x7e => out.push(byte as char),
            other => out.push_str(&format!("\\{other:03o}")),
        }
    }
    out
}

/// Formats one paper millimetre in PDF user space at fixed four-decimal precision.
fn pt(value: f64) -> String {
    let value = value * MM_TO_PT;
    format!("{value:.4}")
}

/// Paints every path as move/line operators translated by `origin_mm`, the paper point
/// mapped onto the page's lower-left corner; each path sets and clears its own dash pattern.
/// A stroke whose start continues the previous stroke's end joins it into one polyline, so
/// a dash pattern runs along a tessellated circle or arc instead of restarting at every
/// chord and painting the circle solid (#490 finding 1).
fn content_stream(paths: &[PagePath], origin_mm: [f64; 2]) -> String {
    let mut path = String::new();
    for group in paths {
        if let Some(dash) = group.dash_mm {
            let pattern = dash
                .iter()
                .map(|value| pt(*value))
                .collect::<Vec<_>>()
                .join(" ");
            path.push_str(&format!("[{pattern}] 0 d\n"));
        }
        let mut pen: Option<(String, String)> = None;
        for [start, end] in &group.strokes {
            let [sx, sy] = *start;
            let [ex, ey] = *end;
            let from = (pt(sx - origin_mm[0]), pt(sy - origin_mm[1]));
            let to = (pt(ex - origin_mm[0]), pt(ey - origin_mm[1]));
            // A closing stroke that only repeats the point the run already reached prints a
            // duplicated point and advances nothing.
            if pen.as_ref() == Some(&from) && from == to {
                continue;
            }
            if pen.as_ref() == Some(&from) {
                path.push_str(&format!("{} {} l\n", to.0, to.1));
            } else {
                path.push_str(&format!("{} {} m {} {} l\n", from.0, from.1, to.0, to.1));
            }
            pen = Some(to);
        }
        path.push_str("S\n");
        if group.dash_mm.is_some() {
            path.push_str("[] 0 d\n");
        }
    }
    format!("{:.4} w 1 J 1 j 0 0 0 RG\n{path}", STROKE_MM * MM_TO_PT)
}

/// Writes one deterministic single-page PDF whose page box is `width_mm` by `height_mm`
/// and whose painted paths are mapped from paper millimetres through `origin_mm`. The
/// document carries no `/ID`, clock or random field; the byte cap matches the SVG export.
pub(super) fn write_page(
    width_mm: f64,
    height_mm: f64,
    paths: &[PagePath],
    origin_mm: [f64; 2],
    title: &str,
    keywords: &str,
) -> Result<Vec<u8>, CommandError> {
    let content = content_stream(paths, origin_mm);
    let objects = [
        String::from("<< /Type /Catalog /Pages 2 0 R >>"),
        String::from("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << >> \
             /Contents 4 0 R >>",
            pt(width_mm),
            pt(height_mm),
        ),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
        format!(
            "<< /Title ({}) /Keywords ({}) /Producer (BeamCraft {}) >>",
            escape_string(title),
            escape_string(keywords),
            PDF_EXPORT_CONTRACT,
        ),
    ];
    let mut pdf = String::from("%PDF-1.4\n%\u{e2}\u{e3}\u{cf}\u{d3}\n");
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.push_str(&format!("{} 0 obj\n{object}\nendobj\n", index + 1));
    }
    let start_xref = pdf.len();
    pdf.push_str("xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets {
        pdf.push_str(&format!("{offset:010} 00000 n \n"));
    }
    pdf.push_str(&format!(
        "trailer\n<< /Size 6 /Root 1 0 R /Info 5 0 R >>\nstartxref\n{start_xref}\n%%EOF\n"
    ));
    if pdf.len() > MAX_EXPORT_BYTES {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "drawing_pdf",
            "the exported drawing exceeds its byte cap",
        ));
    }
    Ok(pdf.into_bytes())
}

/// Renders the exact resolved published geometry as a deterministic stroke-only PDF bound
/// to `version`, on a tight page with the SVG export's margin, with every drawn object on
/// a visible layer. Unresolved geometry, unbounded coordinates and an over-cap result
/// refuse.
pub fn export(
    drawing: &Drawing,
    version: DocumentVersionId,
    projection: &Projection,
) -> Result<Vec<u8>, CommandError> {
    let (groups, _) = stroke_groups(projection)?;
    let drafting = drafting_layers(drawing)?;
    let [min_x, min_y, max_x, max_y] = bounds(
        groups
            .iter()
            .flatten()
            .chain(drafting.iter().flat_map(|layer| layer.strokes.iter())),
    );
    let origin = [min_x - MARGIN_MM, min_y - MARGIN_MM];
    let width = max_x - min_x + 2.0 * MARGIN_MM;
    let height = max_y - min_y + 2.0 * MARGIN_MM;
    let spec = drawing.spec();
    let keywords = format!(
        "{} drawing={} view={} dimension={} version={version} source_revision={} \
         source_manifest={} algorithm={}",
        PDF_EXPORT_CONTRACT,
        spec.id,
        spec.view.id,
        spec.dimension.id,
        spec.source.revision,
        spec.source.manifest,
        ALGORITHM,
    );
    let mut paths: Vec<PagePath> = groups
        .iter()
        .map(|group| PagePath {
            strokes: group.clone(),
            dash_mm: None,
        })
        .collect();
    paths.extend(drafting.iter().map(|layer| PagePath {
        strokes: layer.strokes.clone(),
        dash_mm: super::svg::dash_array(layer.line_type),
    }));
    write_page(width, height, &paths, origin, &spec.name, &keywords)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::drawing::projection::{
        BodyProjection, DimensionGeometry, DimensionProjection, ProjectedEdge, UnresolvedReason,
    };
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
        drawing_with_strokes(name, None)
    }

    fn drawing_with_strokes(name: &str, drafting: Option<DraftingSpec>) -> Drawing {
        let (drafting, layers) = drafting
            .map(|spec| {
                (
                    vec![spec.clone()],
                    vec![LayerSpec {
                        id: spec.layer,
                        name: "Notes".to_owned(),
                        visible: true,
                    }],
                )
            })
            .unwrap_or_default();
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
            drafting,
            layers,
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
    fn bytes_are_deterministic_structured_and_font_free() {
        let drawing = drawing("Member projection");
        let version = DocumentVersionId::parse(C).unwrap();
        let first = export(&drawing, version, &projection("1000")).unwrap();
        let second = export(&drawing, version, &projection("1000")).unwrap();
        assert_eq!(first, second);
        assert!(first.len() <= MAX_EXPORT_BYTES);
        let text = std::str::from_utf8(&first).unwrap();
        assert!(text.starts_with("%PDF-1.4\n"));
        assert!(text.ends_with("%%EOF\n"));
        assert!(text.contains(&format!("version={version}")));
        // The tight page carries the label glyphs, so metadata text stays out of the page:
        // the model-mm label is drawn as strokes, never written as font text.
        assert!(!text.contains("label_mm"));
        // The fixture strokes span 20.7 x 25 mm with the shared margin, in points; the
        // centred digit 1's flag reaches left of the old right-column-only glyph.
        assert!(text.contains("/MediaBox [0 0 58.6772 70.8661]"));
        assert!(text.contains("0.7087 w"));
        // No font anywhere: the glyphs are the shared original stroke geometry.
        assert!(!text.contains("/Font"));
        // The cross-reference table points at every object header exactly.
        let start = text
            .split("startxref\n")
            .nth(1)
            .and_then(|tail| tail.lines().next())
            .and_then(|value| value.parse::<usize>().ok())
            .expect("startxref parses");
        assert!(text[start..].starts_with("xref\n"));
        let entries: Vec<usize> = text[start..]
            .lines()
            .skip(3)
            .take(5)
            .map(|line| line[..10].parse::<usize>().unwrap())
            .collect();
        for (index, offset) in entries.into_iter().enumerate() {
            assert!(text[offset..].starts_with(&format!("{} 0 obj", index + 1)));
        }
        // A different label changes the glyph strokes, so the bytes are content-bound.
        assert_ne!(
            export(&drawing, version, &projection("995")).unwrap(),
            first
        );
    }

    #[test]
    fn a_dashed_circle_sets_its_dash_once_and_draws_one_run() {
        // #490 finding 1 in the PDF surface: the tessellated ring's ~1 mm chords each
        // opened their own subpath, and PDF restarts the dash pattern at every `m`, so
        // dashed circles exported solid. The chords now chain one polyline: the dashed
        // group's segment between its dash setting and the stroke carries one `m`.
        let drawing = drawing_with_strokes(
            "Ring",
            Some(DraftingSpec {
                id: DraftingEntityId::parse("01900000-0000-7000-8000-000000000006").unwrap(),
                geometry: DraftingGeometry::Circle {
                    center: [45.0, 160.0],
                    radius: 12.0,
                },
                layer: DraftingEntityId::parse("01900000-0000-7000-8000-000000000005").unwrap(),
                line_type: LineType::Dashed,
            }),
        );
        let version = DocumentVersionId::parse(C).unwrap();
        let bytes = export(&drawing, version, &projection("1000")).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let dashed = text
            .split("[8.5039 8.5039] 0 d\n")
            .nth(1)
            .unwrap_or_default();
        let dashed = dashed.split("\nS\n").next().unwrap_or_default();
        assert!(
            !dashed.is_empty(),
            "the dashed drafting group is in the content stream"
        );
        assert_eq!(
            dashed.matches(" m ").count(),
            1,
            "the ring is one polyline: {dashed}"
        );
        assert!(dashed.matches(" l\n").count() >= 16, "{dashed}");
        // The pattern is cleared after the dashed group, so later groups stay continuous.
        assert!(text.contains("[] 0 d\n"));
    }

    #[test]
    fn unresolved_and_out_of_range_geometry_refuse() {
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
    }

    #[test]
    fn textual_metadata_is_escaped_in_pdf_strings() {
        let drawing = drawing("a<b>&\"c'");
        let version = DocumentVersionId::parse(C).unwrap();
        let bytes = export(&drawing, version, &projection("10.5")).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.contains("/Title (a<b>&\"c')"));
        assert!(text.contains("/Keywords ("));
    }
}
