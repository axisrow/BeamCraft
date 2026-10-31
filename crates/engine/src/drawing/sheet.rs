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

use super::svg::{
    MAX_EXPORT_BYTES, escape, normalized, path, stroke_groups, text_strokes, text_width,
};
use super::{Drawing, SheetSpec};
use crate::drawing::projection::{BodyProjection, DetailProjection, Projection, SectionProjection};
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{DocumentVersionId, DrawingViewId};

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
/// Gap between a view's lowest stroke and its annotation line below it.
const VIEW_TITLE_GAP_MM: f64 = 3.0;
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

/// One view's annotation line: the authored title and the display scale, e.g.
/// "SECTION A-A 1:50"; a view without a title renders its scale alone.
fn annotation(title: &Option<String>, scale_denominator: f64) -> String {
    let scale = format!("1:{scale_denominator}");
    match title {
        Some(title) => format!("{title} {scale}"),
        None => scale,
    }
}

/// One view's rendered sheet contribution: its id, geometry strokes and annotation strokes,
/// keyed for the SVG `data-view` paths.
type RenderedView = (String, Vec<[[f64; 2]; 2]>, Vec<[[f64; 2]; 2]>);

/// Renders one view of the sheet: its resolved geometry strokes plus the annotation line
/// (title and scale) centred below the geometry's lowest stroke — below the view origin
/// when the geometry is empty, as for a section whose cut misses the member box. The
/// geometry and the annotation are validated and normalized; the id keys the SVG paths.
fn render_view(
    id: &DrawingViewId,
    title: &Option<String>,
    scale_denominator: f64,
    origin: [f64; 2],
    geometry: Vec<[[f64; 2]; 2]>,
) -> Result<RenderedView, CommandError> {
    let strokes = normalized(geometry)?;
    let mut low = [f64::INFINITY; 2];
    let mut high = [f64::NEG_INFINITY; 2];
    for [start, end] in &strokes {
        for [x, y] in [*start, *end] {
            low[0] = low[0].min(x);
            low[1] = low[1].min(y);
            high[0] = high[0].max(x);
            high[1] = high[1].max(y);
        }
    }
    let label = annotation(title, scale_denominator);
    // An empty geometry annotates at the view origin instead of a fabricated bound.
    let (centre_x, bottom) = if low[0] <= high[0] {
        ((low[0] + high[0]) / 2.0, low[1])
    } else {
        (origin[0], origin[1])
    };
    let place = [
        centre_x - text_width(&label) / 2.0,
        bottom - VIEW_TITLE_GAP_MM,
    ];
    Ok((
        id.to_string(),
        strokes,
        normalized(text_strokes(&label, place))?,
    ))
}

/// Renders the exact resolved published geometry of every view as deterministic
/// stroke-only sheet SVG bytes. The document must carry an admitted sheet, the primary
/// projection must be fully resolved, and one resolved body must be supplied per placed
/// view, section and detail in document order; anything unresolved, out of the frame or
/// over the byte cap refuses. Every view is annotated below its geometry with its title
/// and display scale.
pub fn export_svg(
    drawing: &Drawing,
    version: DocumentVersionId,
    primary: &Projection,
    placed_bodies: &[BodyProjection],
    section_bodies: &[SectionProjection],
    detail_bodies: &[DetailProjection],
) -> Result<Vec<u8>, CommandError> {
    let Some(sheet) = drawing.sheet() else {
        return Err(invalid("the document carries no sheet"));
    };
    if placed_bodies.len() != drawing.views().len()
        || section_bodies.len() != drawing.sections().len()
        || detail_bodies.len() != drawing.details().len()
    {
        return Err(invalid(
            "a sheet render needs one resolved body per placed view, section and detail",
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
    // Every view renders its resolved geometry and its annotation line. The primary view's
    // geometry stays in the dimension groups below, so only its annotation is kept here; the
    // placed views, sections and details render as their own data-view paths.
    let spec = drawing.spec();
    let BodyProjection::Resolved(primary_edges) = &primary.body else {
        return Err(invalid("unresolved drawing geometry cannot be exported"));
    };
    let (_, _, primary_title) = render_view(
        &spec.view.id,
        &spec.view.title,
        spec.view.scale_denominator,
        spec.view.paper_origin_mm,
        primary_edges.iter().map(|edge| edge.paper_mm).collect(),
    )?;
    let mut views = Vec::new();
    for (view, body) in drawing.views().iter().zip(placed_bodies) {
        let BodyProjection::Resolved(edges) = body else {
            return Err(invalid("unresolved drawing geometry cannot be exported"));
        };
        views.push(render_view(
            &view.id,
            &view.title,
            view.scale_denominator,
            view.paper_origin_mm,
            edges.iter().map(|edge| edge.paper_mm).collect(),
        )?);
    }
    for (section, projection) in drawing.sections().iter().zip(section_bodies) {
        let SectionProjection::Resolved(geometry) = projection else {
            return Err(invalid("unresolved drawing geometry cannot be exported"));
        };
        let mut strokes: Vec<[[f64; 2]; 2]> =
            geometry.edges.iter().map(|edge| edge.paper_mm).collect();
        // The exact cut contour closes along its boundary order.
        let contour = &geometry.cut_paper_mm;
        for (a, b) in contour.iter().zip(contour.iter().skip(1)) {
            strokes.push([*a, *b]);
        }
        if let [first, .., last] = contour.as_slice() {
            strokes.push([*last, *first]);
        }
        views.push(render_view(
            &section.id,
            &section.title,
            section.scale_denominator,
            section.paper_origin_mm,
            strokes,
        )?);
    }
    for (detail, projection) in drawing.details().iter().zip(detail_bodies) {
        let DetailProjection::Resolved(geometry) = projection else {
            return Err(invalid("unresolved drawing geometry cannot be exported"));
        };
        views.push(render_view(
            &detail.id,
            &detail.title,
            detail.scale_denominator,
            detail.paper_origin_mm,
            geometry.edges.iter().map(|edge| edge.paper_mm).collect(),
        )?);
    }
    let [body, extension, line, arrows, label] = groups;
    let everything: Vec<&[[f64; 2]; 2]> = frame
        .iter()
        .chain(block.iter())
        .chain(views.iter().flat_map(|(_, strokes, _)| strokes.iter()))
        .chain(views.iter().flat_map(|(_, _, title)| title.iter()))
        .chain(primary_title.iter())
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
    for (id, strokes, title) in &views {
        text.push_str(&format!(
            "<path class=\"body\" data-view=\"{}\" d=\"{}\"/>\n",
            escape(id),
            path(strokes),
        ));
        text.push_str(&format!(
            "<path class=\"viewtitle\" data-view=\"{}\" d=\"{}\"/>\n",
            escape(id),
            path(title),
        ));
    }
    text.push_str(&format!(
        "<path class=\"viewtitle\" data-view=\"{}\" d=\"{}\"/>\n",
        escape(&spec.view.id.to_string()),
        path(&primary_title),
    ));
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
    use crate::drawing::projection::{
        DetailGeometry, DetailProjection, DimensionGeometry, DimensionProjection, ProjectedEdge,
        SectionGeometry, SectionProjection, UnresolvedReason,
    };
    use crate::drawing::{
        CutSide, DEFAULT_SCALE_DENOMINATOR, DetailSpec, DimensionSpec, Drawing, DrawingSpec, Plane,
        SectionSpec, SourceModelPin, ViewSpec,
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
            views: placed
                .iter()
                .map(|id| ViewSpec {
                    id: DrawingViewId::parse(id).unwrap(),
                    plane: Plane::Top,
                    scale_denominator: DEFAULT_SCALE_DENOMINATOR,
                    paper_origin_mm: [60.0, 40.0],
                    title: None,
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
        let first = export_svg(&drawing, version, &projection(), &bodies, &[], &[]).unwrap();
        let second = export_svg(&drawing, version, &projection(), &bodies, &[], &[]).unwrap();
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
                &bodies,
                &[],
                &[]
            )
            .unwrap(),
            first
        );
        let retitled = sheet_drawing(Sheet::A4, &[D]);
        let mut spec = retitled.spec().clone();
        spec.sheet.as_mut().unwrap().title.project = "Harbour".to_owned();
        let retitled = retitled.with_sheet(spec.sheet.clone().unwrap()).unwrap();
        assert_ne!(
            export_svg(&retitled, version, &projection(), &bodies, &[], &[]).unwrap(),
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
            export_svg(&loose, version, &projection(), &[], &[], &[])
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
            export_svg(&drawing, version, &unresolved, &[], &[], &[])
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
                &[BodyProjection::Unresolved(UnresolvedReason::MissingOwner)],
                &[],
                &[],
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidArgument
        );
        // A body count that disagrees with the placed views, sections or details refuses.
        assert_eq!(
            export_svg(&drawing, version, &projection(), &[], &[], &[])
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            export_svg(
                &drawing,
                version,
                &projection(),
                &bodies,
                &[section_body(Vec::new(), Vec::new())],
                &[],
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidArgument
        );
        // Content beyond the frame refuses instead of being clipped.
        let far = vec![placed_body([900.0, 45.0])];
        assert_eq!(
            export_svg(&drawing, version, &projection(), &far, &[], &[])
                .unwrap_err()
                .code,
            ErrorCode::LimitExceeded
        );
        // A frame that leaves no room for the title block refuses before drawing.
        let mut tight = drawing.spec().clone();
        tight.sheet.as_mut().unwrap().frame.inset_mm = 100.0;
        let tight = drawing.with_sheet(tight.sheet.unwrap()).unwrap();
        assert_eq!(
            export_svg(&tight, version, &projection(), &bodies, &[], &[])
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
    }

    /// A resolved section projection: the visible-half edges plus the closed cut contour.
    fn section_body(edges: Vec<ProjectedEdge>, contour: Vec<[f64; 2]>) -> SectionProjection {
        SectionProjection::Resolved(SectionGeometry {
            edges,
            cut_paper_mm: contour,
        })
    }

    /// A resolved detail projection over the given mapped edges.
    fn detail_body(edges: Vec<ProjectedEdge>) -> DetailProjection {
        DetailProjection::Resolved(DetailGeometry {
            edges,
            region_center_mm: [70.0, 45.0],
            region_radius_mm: 5.0,
        })
    }

    fn section_spec(id: &str, title: Option<&str>) -> SectionSpec {
        SectionSpec {
            id: DrawingViewId::parse(id).unwrap(),
            plane: Plane::Right,
            cut_mm: 1500.0,
            side: CutSide::Positive,
            scale_denominator: 20.0,
            paper_origin_mm: [90.0, 40.0],
            title: title.map(str::to_owned),
        }
    }

    fn detail_spec(id: &str, parent: &str, title: Option<&str>) -> DetailSpec {
        DetailSpec {
            id: DrawingViewId::parse(id).unwrap(),
            parent: DrawingViewId::parse(parent).unwrap(),
            center_paper_mm: [15.0, 20.0],
            radius_mm: 5.0,
            scale_denominator: 20.0,
            paper_origin_mm: [120.0, 40.0],
            title: title.map(str::to_owned),
        }
    }

    fn section_edge(x: f64, y: f64) -> ProjectedEdge {
        ProjectedEdge {
            feature: FeatureId::parse(&format!("fid1:{MEMBER}:member:e.x0.y0")).unwrap(),
            paper_mm: [[x, y], [x + 10.0, y]],
        }
    }

    #[test]
    fn every_view_is_annotated_with_its_title_and_display_scale() {
        let version = DocumentVersionId::parse(C).unwrap();
        const SECTION: &str = "01900000-0000-7000-8000-000000000341";
        const DETAIL: &str = "01900000-0000-7000-8000-000000000342";
        let base = sheet_drawing(Sheet::A4, &[D]);
        let mut s = base.spec().clone();
        s.view.title = Some("FRONT".to_owned());
        s.views[0].title = Some("PLAN".to_owned());
        s.sections = vec![section_spec(SECTION, Some("SECTION A-A"))];
        s.details = vec![detail_spec(DETAIL, B, None)];
        let drawing = Drawing::admit(s).unwrap();
        let bodies = vec![placed_body([65.0, 45.0])];
        let sections = vec![section_body(
            vec![section_edge(95.0, 45.0)],
            vec![[95.0, 42.0], [105.0, 42.0], [105.0, 48.0]],
        )];
        let details = vec![detail_body(vec![section_edge(125.0, 45.0)])];
        let bytes = export_svg(
            &drawing,
            version,
            &projection(),
            &bodies,
            &sections,
            &details,
        )
        .unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        // Every view — primary, placed, section and detail — carries one annotation path.
        assert_eq!(text.matches("class=\"viewtitle\"").count(), 4);
        for id in [B, D, SECTION, DETAIL] {
            assert!(text.contains(&format!("data-view=\"{id}\"")), "{id}");
        }
        // The same document renders the same bytes; an annotation change moves them.
        assert_eq!(
            bytes,
            export_svg(
                &drawing,
                version,
                &projection(),
                &bodies,
                &sections,
                &details
            )
            .unwrap()
        );
        let mut retitled = drawing.spec().clone();
        retitled.views[0].title = Some("PLAN VIEW".to_owned());
        let retitled = Drawing::admit(retitled).unwrap();
        assert_ne!(
            export_svg(
                &retitled,
                version,
                &projection(),
                &bodies,
                &sections,
                &details
            )
            .unwrap(),
            bytes
        );
        // A scale change alone moves the annotation bytes of exactly that view: the same
        // resolved geometry under a different declared scale annotates differently.
        let mut scaled = drawing.spec().clone();
        scaled.sections[0].scale_denominator = 10.0;
        let scaled = Drawing::admit(scaled).unwrap();
        assert_ne!(
            export_svg(
                &scaled,
                version,
                &projection(),
                &bodies,
                &sections,
                &details
            )
            .unwrap(),
            bytes
        );
        // Unresolved derived views refuse exactly like unresolved placed views.
        for refusal in [
            export_svg(
                &drawing,
                version,
                &projection(),
                &bodies,
                &[SectionProjection::Unresolved(
                    UnresolvedReason::MissingOwner,
                )],
                &details,
            ),
            export_svg(
                &drawing,
                version,
                &projection(),
                &bodies,
                &sections,
                &[DetailProjection::Unresolved(UnresolvedReason::MissingOwner)],
            ),
        ] {
            assert_eq!(refusal.unwrap_err().code, ErrorCode::InvalidArgument);
        }
        // An empty resolved section (the cut misses the member box) still annotates,
        // at the view origin instead of a fabricated bound.
        let empty = vec![section_body(Vec::new(), Vec::new())];
        let mut origin_based = drawing.spec().clone();
        origin_based.details = Vec::new();
        let origin_based = Drawing::admit(origin_based).unwrap();
        let annotated =
            export_svg(&origin_based, version, &projection(), &bodies, &empty, &[]).unwrap();
        assert!(
            String::from_utf8(annotated)
                .unwrap()
                .contains(&format!("data-view=\"{SECTION}\""))
        );
    }

    #[test]
    fn an_annotation_outside_the_frame_refuses_instead_of_clipping() {
        let version = DocumentVersionId::parse(C).unwrap();
        // The body stroke sits inside the frame, but its annotation line below falls outside.
        let drawing = sheet_drawing(Sheet::A4, &[D]);
        let near = vec![placed_body([65.0, 12.0])];
        assert_eq!(
            export_svg(&drawing, version, &projection(), &near, &[], &[])
                .unwrap_err()
                .code,
            ErrorCode::LimitExceeded
        );
    }
}
