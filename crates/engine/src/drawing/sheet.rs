// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D05: the whole-sheet renderer of one exact published document version.
//!
//! A sheet renders the admitted furniture — frame and title block — and every resolved
//! view of the document side by side on the chosen fixed paper. Every stroke, glyph and
//! line is contributor-original vector geometry, exactly as in the single-view export: no
//! font, image or external resource is embedded or referenced. Output bytes depend only on
//! the verified published inputs; the caller's model and document are never touched. A
//! document without a sheet keeps the earlier loose-paper export, and content that does not
//! fit the frame refuses instead of being rescaled or clipped.

use super::svg::{MAX_EXPORT_BYTES, escape, normalized, path, stroke_groups, text_strokes};
use super::{Drawing, SheetSpec};
use crate::drawing::projection::{BodyProjection, Projection};
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::DocumentVersionId;

/// Title block width in millimetres, anchored inside the frame's bottom-right corner.
const TITLE_BLOCK_WIDTH_MM: f64 = 160.0;
/// Title block height in millimetres: four equal rows.
const TITLE_BLOCK_HEIGHT_MM: f64 = 44.0;
/// Width of the fixed label column inside the title block.
const TITLE_BLOCK_LABEL_MM: f64 = 30.0;
/// Number of title block rows, bottom to top: scale, author, drawing, project.
const TITLE_BLOCK_ROWS: usize = 4;
/// Gap between a title block cell edge and its text.
const TITLE_BLOCK_PAD_MM: f64 = 2.0;
/// Exact contract of the whole-sheet export, independent of the loose-paper contract.
pub const SHEET_EXPORT_CONTRACT: &str = "beamcraft-drawing-sheet/1";

fn invalid(message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::InvalidArgument, "drawing_sheet", message)
}

/// One title block row: fixed label and the derived or authored value rendered beside it.
fn title_rows(sheet: &SheetSpec, name: &str, scale: String) -> Vec<(String, String)> {
    vec![
        ("SCALE".to_owned(), format!("1:{scale}")),
        ("AUTHOR".to_owned(), sheet.title.author.clone()),
        ("DRAWING".to_owned(), name.to_owned()),
        ("PROJECT".to_owned(), sheet.title.project.clone()),
    ]
}

/// Renders the exact resolved published geometry of every view as deterministic
/// stroke-only sheet SVG bytes. The document must carry an admitted sheet, the primary
/// projection must be fully resolved, and one resolved body must be supplied per placed
/// view in document order; anything unresolved, out of the frame or over the byte cap
/// refuses.
pub fn export_svg(
    drawing: &Drawing,
    version: DocumentVersionId,
    primary: &Projection,
    placed_bodies: &[BodyProjection],
) -> Result<Vec<u8>, CommandError> {
    let Some(sheet) = drawing.sheet() else {
        return Err(invalid("the document carries no sheet"));
    };
    if placed_bodies.len() != drawing.views().len() {
        return Err(invalid(
            "a sheet render needs one resolved body per placed view",
        ));
    }
    let (groups, _) = stroke_groups(primary)?;
    let [width_mm, height_mm] = sheet.size.size_mm();
    let inset = sheet.frame.inset_mm;
    let [inner_x, inner_y] = [width_mm - inset, height_mm - inset];
    // The furniture must fit the frame interior; a sheet whose interior cannot hold the
    // title block refuses before any stroke is drawn.
    if inner_x - inset < TITLE_BLOCK_WIDTH_MM || inner_y - inset < TITLE_BLOCK_HEIGHT_MM {
        return Err(invalid(
            "the sheet interior is smaller than the title block",
        ));
    }
    let frame = normalized(vec![
        // Frame: one rectangle from the uniform inset to the opposite edges.
        [[inset, inset], [inner_x, inset]],
        [[inner_x, inset], [inner_x, inner_y]],
        [[inner_x, inner_y], [inset, inner_y]],
        [[inset, inner_y], [inset, inset]],
    ])?;
    let block_x = inner_x - TITLE_BLOCK_WIDTH_MM;
    let row_height = TITLE_BLOCK_HEIGHT_MM / TITLE_BLOCK_ROWS as f64;
    let mut block = Vec::new();
    for row in 1..TITLE_BLOCK_ROWS {
        let y = inset + row as f64 * row_height;
        block.push([[block_x, y], [inner_x, y]]);
    }
    block.push([[block_x, inset], [block_x, inset + TITLE_BLOCK_HEIGHT_MM]]);
    block.push([
        [inner_x - TITLE_BLOCK_LABEL_MM, inset],
        [
            inner_x - TITLE_BLOCK_LABEL_MM,
            inset + TITLE_BLOCK_HEIGHT_MM,
        ],
    ]);
    let rows = title_rows(
        sheet,
        &drawing.spec().name,
        format!("{}", drawing.spec().view.scale_denominator),
    );
    for (index, (label, value)) in rows.iter().enumerate() {
        let baseline = inset + index as f64 * row_height + (row_height - 3.5) / 2.0;
        block.extend(text_strokes(
            label,
            [block_x + TITLE_BLOCK_PAD_MM, baseline],
        ));
        block.extend(text_strokes(
            value,
            [
                block_x + TITLE_BLOCK_LABEL_MM + TITLE_BLOCK_PAD_MM,
                baseline,
            ],
        ));
    }
    let block = normalized(block)?;
    // Placed views draw their resolved wireframes in document order, then the primary
    // view's dimension groups keep the classes of the loose-paper export.
    let mut placed = Vec::new();
    for (view, body) in drawing.views().iter().zip(placed_bodies) {
        let BodyProjection::Resolved(edges) = body else {
            return Err(invalid("unresolved drawing geometry cannot be exported"));
        };
        let strokes = normalized(edges.iter().map(|edge| edge.paper_mm).collect())?;
        placed.push((view.id.to_string(), strokes));
    }
    let [body, extension, line, arrows, label] = groups;
    let everything: Vec<&[[f64; 2]; 2]> = frame
        .iter()
        .chain(block.iter())
        .chain(placed.iter().flat_map(|(_, strokes)| strokes.iter()))
        .chain(body.iter())
        .chain(extension.iter())
        .chain(line.iter())
        .chain(arrows.iter())
        .chain(label.iter())
        .collect();
    // Every stroke must lie inside the frame; content that does not fit refuses instead of
    // being silently rescaled or clipped.
    for group in everything.iter() {
        for [x, y] in group.iter() {
            if *x < inset || *x > inner_x || *y < inset || *y > inner_y {
                return Err(CommandError::new(
                    ErrorCode::LimitExceeded,
                    "drawing_sheet",
                    "drawing content does not fit the sheet frame",
                ));
            }
        }
    }
    let spec = drawing.spec();
    let metadata = format!(
        "{SHEET_EXPORT_CONTRACT} drawing={} version={} sheet={} source_revision={} \
         source_manifest={} scale_denominator={}",
        escape(&spec.id.to_string()),
        escape(&version.to_string()),
        sheet.size.as_str(),
        escape(&spec.source.revision.to_string()),
        escape(&spec.source.manifest.to_string()),
        spec.view.scale_denominator,
    );
    let mut text = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <svg xmlns=\"http://www.w3.org/2000/svg\" version=\"1.1\" width=\"{width_mm:.2}mm\" \
         height=\"{height_mm:.2}mm\" viewBox=\"0 0 {width_mm:.2} {height_mm:.2}\">\n\
         <title>{}</title>\n<metadata>{metadata}</metadata>\n\
         <g fill=\"none\" stroke=\"#000000\" stroke-width=\"0.25\" stroke-linecap=\"round\">\n\
         <path class=\"frame\" d=\"{}\"/>\n<path class=\"titleblock\" d=\"{}\"/>\n",
        escape(&spec.name),
        path(&frame),
        path(&block),
    );
    for (id, strokes) in &placed {
        text.push_str(&format!(
            "<path class=\"body\" data-view=\"{}\" d=\"{}\"/>\n",
            escape(id),
            path(strokes),
        ));
    }
    text.push_str(&format!(
        "<path class=\"body\" d=\"{}\"/>\n<path class=\"extension\" d=\"{}\"/>\n\
         <path class=\"dimension\" d=\"{}\"/>\n<path class=\"arrow\" d=\"{}\"/>\n\
         <path class=\"label\" d=\"{}\"/>\n</g>\n</svg>\n",
        path(&body),
        path(&extension),
        path(&line),
        path(&arrows),
        path(&label),
    ));
    if text.len() > MAX_EXPORT_BYTES {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "drawing_sheet",
            "the exported sheet exceeds its byte cap",
        ));
    }
    Ok(text.into_bytes())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::drawing::projection::{DimensionGeometry, DimensionProjection, ProjectedEdge};
    use crate::drawing::{
        DEFAULT_SCALE_DENOMINATOR, DimensionSpec, Drawing, DrawingSpec, Plane, SourceModelPin,
        ViewSpec,
    };
    use crate::query::feature::FeatureId;
    use beamcraft_model::digest::ContentDigest;
    use beamcraft_model::id::{
        DimensionId, DrawingId, DrawingViewId, ElementId, ProjectId, RevisionId,
    };
    use beamcraft_model::sheet::{Sheet, SheetFrame, SheetSpec, TitleBlock};

    const A: &str = "01900000-0000-7000-8000-000000000001";
    const B: &str = "01900000-0000-7000-8000-000000000002";
    const C: &str = "01900000-0000-7000-8000-000000000003";
    const D: &str = "01900000-0000-7000-8000-000000000004";
    const MEMBER: &str = "01900000-0000-7000-8000-000000000005";

    fn sheet(size: Sheet) -> SheetSpec {
        SheetSpec {
            size,
            frame: SheetFrame { inset_mm: 10.0 },
            title: TitleBlock {
                project: "Bridge".to_owned(),
                author: "Drafter".to_owned(),
            },
        }
    }

    fn sheet_drawing(size: Sheet, placed: &[&str]) -> Drawing {
        Drawing::admit(DrawingSpec {
            id: DrawingId::parse(A).unwrap(),
            name: "Member drawing".to_owned(),
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
                paper_origin_mm: [30.0, 40.0],
            },
            dimension: DimensionSpec {
                id: DimensionId::parse(C).unwrap(),
                anchors: ["v.x0.y0.z0", "v.x0.y0.z1"]
                    .map(|role| FeatureId::parse(&format!("fid1:{MEMBER}:member:{role}")).unwrap()),
                paper_offset_mm: 10.0,
                decimal_places: 0,
            },
            drafting: Vec::new(),
            views: placed
                .iter()
                .map(|id| ViewSpec {
                    id: DrawingViewId::parse(id).unwrap(),
                    plane: Plane::Top,
                    scale_denominator: DEFAULT_SCALE_DENOMINATOR,
                    paper_origin_mm: [60.0, 40.0],
                })
                .collect(),
            sections: Vec::new(),
            details: Vec::new(),
            sheet: Some(sheet(size)),
            layers: Vec::new(),
        })
        .unwrap()
    }

    fn projection() -> Projection {
        Projection {
            body: BodyProjection::Resolved(vec![ProjectedEdge {
                feature: FeatureId::parse(&format!("fid1:{MEMBER}:member:e.x0.y0")).unwrap(),
                paper_mm: [[35.0, 45.0], [45.0, 45.0]],
            }]),
            dimension: DimensionProjection::Resolved(DimensionGeometry {
                anchors_paper_mm: [[35.0, 45.0], [45.0, 45.0]],
                line_paper_mm: [[35.0, 55.0], [45.0, 55.0]],
                projected_length_mm: 1000.0,
                label_mm: "1000".to_owned(),
            }),
        }
    }

    fn placed_body(origin: [f64; 2]) -> BodyProjection {
        BodyProjection::Resolved(vec![ProjectedEdge {
            feature: FeatureId::parse(&format!("fid1:{MEMBER}:member:e.x0.z0")).unwrap(),
            paper_mm: [origin, [origin[0] + 10.0, origin[1]]],
        }])
    }

    #[test]
    fn the_same_sheet_reproduces_the_same_output() {
        let drawing = sheet_drawing(Sheet::A4, &[D]);
        let version = DocumentVersionId::parse(C).unwrap();
        let bodies = vec![placed_body([65.0, 45.0])];
        let first = export_svg(&drawing, version, &projection(), &bodies).unwrap();
        let second = export_svg(&drawing, version, &projection(), &bodies).unwrap();
        assert_eq!(first, second);
        assert!(first.len() <= MAX_EXPORT_BYTES);
        let text = std::str::from_utf8(&first).unwrap();
        assert!(text.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(text.ends_with("</svg>\n"));
        assert!(text.contains(&format!("version={version} sheet=a4")));
        assert!(text.contains("class=\"frame\""));
        assert!(text.contains("class=\"titleblock\""));
        assert!(text.contains("data-view"));
        // A different sheet size, title field or scale changes the bytes: the output is
        // bound to the whole sheet, not just the strokes.
        assert_ne!(
            export_svg(
                &sheet_drawing(Sheet::A3, &[D]),
                version,
                &projection(),
                &bodies
            )
            .unwrap(),
            first
        );
        let retitled = sheet_drawing(Sheet::A4, &[D]);
        let mut spec = retitled.spec().clone();
        spec.sheet.as_mut().unwrap().title.project = "Harbour".to_owned();
        let retitled = retitled.with_sheet(spec.sheet.clone().unwrap()).unwrap();
        assert_ne!(
            export_svg(&retitled, version, &projection(), &bodies).unwrap(),
            first
        );
        // Glyph text of the title block is drawn, not embedded as font text.
        assert!(!text.contains("<text"));
        for hostile in ["<script", "<image", "href", "url(", "@import"] {
            assert!(!text.contains(hostile));
        }
    }

    #[test]
    fn a_sheet_refuses_unsupported_inputs_instead_of_approximating() {
        let version = DocumentVersionId::parse(C).unwrap();
        let bodies = vec![placed_body([65.0, 45.0])];
        // A document without a sheet keeps the loose-paper export and refuses here.
        let loose = sheet_drawing(Sheet::A4, &[]).without_sheet();
        assert_eq!(
            export_svg(&loose, version, &projection(), &[])
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        // Unresolved geometry refuses, for the primary and for a placed view alike.
        let unresolved = Projection {
            body: BodyProjection::Resolved(vec![ProjectedEdge {
                feature: FeatureId::parse(&format!("fid1:{MEMBER}:member:e.x0.y0")).unwrap(),
                paper_mm: [[35.0, 45.0], [45.0, 45.0]],
            }]),
            dimension: DimensionProjection::Unresolved(
                crate::drawing::projection::UnresolvedReason::MissingOwner,
            ),
        };
        let drawing = sheet_drawing(Sheet::A4, &[]);
        assert_eq!(
            export_svg(&drawing, version, &unresolved, &[])
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        let drawing = sheet_drawing(Sheet::A4, &[D]);
        assert_eq!(
            export_svg(
                &drawing,
                version,
                &projection(),
                &[BodyProjection::Unresolved(
                    crate::drawing::projection::UnresolvedReason::MissingOwner,
                )],
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidArgument
        );
        // A body count that disagrees with the placed views refuses.
        assert_eq!(
            export_svg(&drawing, version, &projection(), &[])
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        // Content beyond the frame refuses instead of being clipped.
        let far = vec![placed_body([900.0, 45.0])];
        assert_eq!(
            export_svg(&drawing, version, &projection(), &far)
                .unwrap_err()
                .code,
            ErrorCode::LimitExceeded
        );
        // A frame that leaves no room for the title block refuses before drawing.
        let mut tight = drawing.spec().clone();
        tight.sheet.as_mut().unwrap().frame.inset_mm = 100.0;
        let tight = drawing.with_sheet(tight.sheet.unwrap()).unwrap();
        assert_eq!(
            export_svg(&tight, version, &projection(), &bodies)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
    }
}
