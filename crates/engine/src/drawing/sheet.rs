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
    MAX_EXPORT_BYTES, StrokeGroups, circle_strokes, dash_array, drafting_layers, escape,
    flipped_about, normalized, path, path_element, stroke_groups, text_strokes, text_width,
};
use super::{Drawing, SheetSpec};
use crate::drawing::pdf::{self, PagePath};
use crate::drawing::projection::{BodyProjection, DetailProjection, Projection, SectionProjection};
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{DocumentVersionId, DrawingViewId};

/// Title block width in millimetres, anchored inside the frame's bottom-right corner. Public
/// because the sheet editor preview draws the same block the export draws and enforces.
pub const TITLE_BLOCK_WIDTH_MM: f64 = 160.0;
/// Title block height in millimetres: four equal rows, shared with the preview like the width.
pub const TITLE_BLOCK_HEIGHT_MM: f64 = 44.0;
/// Width of the fixed label column inside the title block.
const TITLE_BLOCK_LABEL_MM: f64 = 30.0;
/// Number of title block rows, bottom to top: scale, author, drawing, project.
const TITLE_BLOCK_ROWS: usize = 4;
/// Gap between a title block cell edge and its text.
const TITLE_BLOCK_PAD_MM: f64 = 2.0;
/// Gap between a view's lowest stroke and its annotation line below it. Above the 3.5 mm
/// glyph height, so the title clears the geometry it annotates (visual audit finding 5).
const VIEW_TITLE_GAP_MM: f64 = 5.0;
/// Gap between a detail's parent-side circle and the callout text beside it.
const DETAIL_CALLOUT_GAP_MM: f64 = 1.0;
/// Line pitch of stacked detail callouts: the 3.5 mm glyph height plus a 1.5 mm gap.
const DETAIL_CALLOUT_LINE_MM: f64 = 5.0;
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

/// One rendered sheet layer in draw order: the SVG class, its optional view or drafting
/// key, its authored dash pattern and its paper-millimetre strokes.
pub(super) struct SheetLayer {
    /// The SVG class: frame, titleblock, body, viewtitle or drafting.
    pub class: &'static str,
    /// The placed view, section or detail id, the SVG `data-view` key.
    pub data_view: Option<String>,
    /// The drawn object's id, the SVG `data-drafting` key.
    pub data_drafting: Option<String>,
    /// The authored dash pattern; unbroken when `None`.
    pub dash: Option<&'static [f64]>,
    /// Paper-millimetre strokes of the layer.
    pub strokes: Vec<[[f64; 2]; 2]>,
}

/// The whole rendered sheet in paper millimetres, shared by the SVG and PDF writers so
/// every export surface renders the same fixed sheet.
pub(super) struct SheetLayout {
    /// Paper width.
    pub width_mm: f64,
    /// Paper height.
    pub height_mm: f64,
    /// The contract metadata line both surfaces quote.
    pub metadata: String,
    /// Furniture, view bodies and annotations and drawn objects, in draw order.
    pub layers: Vec<SheetLayer>,
    /// The primary view's dimension groups, keeping the loose-paper classes.
    pub primary: StrokeGroups,
}

/// The `data-view` or `data-drafting` key of one layer, for the SVG path element.
fn layer_key(layer: &SheetLayer) -> Option<(&'static str, &str)> {
    if let Some(id) = &layer.data_drafting {
        return Some(("data-drafting", id));
    }
    layer.data_view.as_deref().map(|id| ("data-view", id))
}

/// Renders one view of the sheet: its resolved geometry strokes plus the annotation line
/// (title and scale) centred below the geometry's lowest stroke — below the view origin
/// when the geometry is empty, as for a section whose cut misses the member box.
/// `dimension` carries the view's own dimension groups when it has one: the annotation
/// must clear them too, so a title never lands inside a dimension's extension lines
/// (visual audit finding 9). The geometry and the annotation are validated and
/// normalized; the id keys the SVG paths.
fn render_view(
    id: &DrawingViewId,
    title: &Option<String>,
    scale_denominator: f64,
    origin: [f64; 2],
    geometry: Vec<[[f64; 2]; 2]>,
    dimension: Option<&StrokeGroups>,
) -> Result<RenderedView, CommandError> {
    let strokes = normalized(geometry)?;
    // The annotation keeps the geometry's own horizontal centre and moves down to clear
    // the view's dimension group as well: a dimension hanging beside or under the view
    // widens the extent, so the centre stays geometry-owned while the bottom clears the
    // whole group (visual audit finding 9).
    let mut low = [f64::INFINITY; 2];
    let mut high = [f64::NEG_INFINITY; 2];
    for [start, end] in strokes.iter() {
        for [x, y] in [*start, *end] {
            low[0] = low[0].min(x);
            low[1] = low[1].min(y);
            high[0] = high[0].max(x);
            high[1] = high[1].max(y);
        }
    }
    let mut bottom = low[1];
    for group in dimension.into_iter().flatten() {
        for stroke in group {
            for [_, y] in [stroke[0], stroke[1]] {
                bottom = bottom.min(y);
            }
        }
    }
    let label = annotation(title, scale_denominator);
    // An empty geometry annotates at the view origin instead of a fabricated bound.
    let (centre_x, bottom) = if low[0] <= high[0] {
        ((low[0] + high[0]) / 2.0, bottom)
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

/// Lays out the exact resolved published geometry of every view plus every drawn object
/// on a visible layer. The document must carry an admitted sheet, the primary projection
/// must be fully resolved, and one resolved body must be supplied per placed view,
/// section and detail in document order; anything unresolved, out of the frame or over
/// the byte cap refuses. Every view is annotated below its geometry with its title and
/// display scale.
fn layout(
    drawing: &Drawing,
    version: DocumentVersionId,
    primary: &Projection,
    placed_bodies: &[BodyProjection],
    section_bodies: &[SectionProjection],
    detail_bodies: &[DetailProjection],
) -> Result<SheetLayout, CommandError> {
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
    // The block closes at the top: the frame owns the bottom, left and right edges.
    block.push([
        [block_x, inset + TITLE_BLOCK_HEIGHT_MM],
        [inner_x, inset + TITLE_BLOCK_HEIGHT_MM],
    ]);
    block.push([[block_x, inset], [block_x, inset + TITLE_BLOCK_HEIGHT_MM]]);
    // The divider follows the label column, so labels and values sit in separate cells
    // and a long value never strikes through its label (visual audit finding 6).
    block.push([
        [block_x + TITLE_BLOCK_LABEL_MM, inset],
        [
            block_x + TITLE_BLOCK_LABEL_MM,
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
    // Placed views draw their resolved wireframes in document order; the primary view's
    // geometry stays in the dimension groups, so only its annotation is a layer here.
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
        // The primary view's dimension hangs under its body; the annotation must clear
        // the extension lines, the line, the arrowheads and the dimension label alike.
        Some(&groups),
    )?;
    let mut layers = vec![
        SheetLayer {
            class: "frame",
            data_view: None,
            data_drafting: None,
            dash: None,
            strokes: frame,
        },
        SheetLayer {
            class: "titleblock",
            data_view: None,
            data_drafting: None,
            dash: None,
            strokes: block,
        },
    ];
    for (view, body) in drawing.views().iter().zip(placed_bodies) {
        let BodyProjection::Resolved(edges) = body else {
            return Err(invalid("unresolved drawing geometry cannot be exported"));
        };
        let (id, strokes, title) = render_view(
            &view.id,
            &view.title,
            view.scale_denominator,
            view.paper_origin_mm,
            edges.iter().map(|edge| edge.paper_mm).collect(),
            None,
        )?;
        layers.push(SheetLayer {
            class: "body",
            data_view: Some(id.clone()),
            data_drafting: None,
            dash: None,
            strokes,
        });
        layers.push(SheetLayer {
            class: "viewtitle",
            data_view: Some(id),
            data_drafting: None,
            dash: None,
            strokes: title,
        });
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
        let (id, body, title) = render_view(
            &section.id,
            &section.title,
            section.scale_denominator,
            section.paper_origin_mm,
            strokes,
            None,
        )?;
        layers.push(SheetLayer {
            class: "body",
            data_view: Some(id.clone()),
            data_drafting: None,
            dash: None,
            strokes: body,
        });
        layers.push(SheetLayer {
            class: "viewtitle",
            data_view: Some(id),
            data_drafting: None,
            dash: None,
            strokes: title,
        });
    }
    for (detail, projection) in drawing.details().iter().zip(detail_bodies) {
        let DetailProjection::Resolved(geometry) = projection else {
            return Err(invalid("unresolved drawing geometry cannot be exported"));
        };
        // The detail draws its region's circular boundary: the ring is part of the view's
        // picture, so the annotation clears it too (visual audit finding 8).
        let mut strokes: Vec<[[f64; 2]; 2]> =
            geometry.edges.iter().map(|edge| edge.paper_mm).collect();
        strokes.extend(circle_strokes(
            geometry.region_center_mm,
            geometry.region_radius_mm,
        )?);
        let (id, body, title) = render_view(
            &detail.id,
            &detail.title,
            detail.scale_denominator,
            detail.paper_origin_mm,
            strokes,
            None,
        )?;
        layers.push(SheetLayer {
            class: "body",
            data_view: Some(id.clone()),
            data_drafting: None,
            dash: None,
            strokes: body,
        });
        layers.push(SheetLayer {
            class: "viewtitle",
            data_view: Some(id),
            data_drafting: None,
            dash: None,
            strokes: title,
        });
    }
    // Every detail marks its region on the parent view: the circular callout that says
    // where the magnified content comes from (visual audit finding 8). The mark belongs
    // to the detail and draws in the parent's paper space, which the sheet shares.
    let mut callouts: Vec<[f64; 4]> = Vec::new();
    for (detail, projection) in drawing.details().iter().zip(detail_bodies) {
        let DetailProjection::Resolved(_) = projection else {
            continue;
        };
        let mut mark = circle_strokes(detail.center_paper_mm, detail.radius_mm)?;
        // The callout names the detail beside its circle, at the circle's upper right, so
        // two marked regions on one parent never leave the reader guessing which detail
        // each one feeds (the recheck of visual audit finding 8). An untitled detail keeps
        // the bare circle: nothing is fabricated.
        if let Some(title) = &detail.title {
            let [cx, cy] = detail.center_paper_mm;
            let reach = detail.radius_mm * std::f64::consts::FRAC_1_SQRT_2 + DETAIL_CALLOUT_GAP_MM;
            let width = text_width(title);
            let mut origin = [cx + reach, cy + reach];
            // Two details of one region (or of overlapping regions) would write their names
            // over each other: a later callout steps up one line until it clears every name
            // already placed. Bounded by the number of details, so it always ends.
            let overlaps = |origin: [f64; 2], placed: &[[f64; 4]]| {
                placed.iter().any(|[lx, ly, hx, hy]| {
                    origin[0] < *hx
                        && origin[0] + width > *lx
                        && origin[1] < *hy
                        && origin[1] + DETAIL_CALLOUT_LINE_MM > *ly
                })
            };
            for _ in 0..=callouts.len() {
                if !overlaps(origin, &callouts) {
                    break;
                }
                origin[1] += DETAIL_CALLOUT_LINE_MM;
            }
            callouts.push([
                origin[0],
                origin[1],
                origin[0] + width,
                origin[1] + DETAIL_CALLOUT_LINE_MM,
            ]);
            mark.extend(text_strokes(title, origin));
        }
        layers.push(SheetLayer {
            class: "detailmark",
            data_view: Some(detail.id.to_string()),
            data_drafting: None,
            dash: None,
            strokes: normalized(mark)?,
        });
    }
    // Drawn objects on visible layers sit above the views they annotate.
    for layer in drafting_layers(drawing)? {
        layers.push(SheetLayer {
            class: "drafting",
            data_view: None,
            data_drafting: Some(layer.id),
            dash: dash_array(layer.line_type),
            strokes: layer.strokes,
        });
    }
    layers.push(SheetLayer {
        class: "viewtitle",
        data_view: Some(spec.view.id.to_string()),
        data_drafting: None,
        dash: None,
        strokes: primary_title,
    });
    // Every stroke must lie inside the frame; content that does not fit refuses instead
    // of being silently rescaled or clipped.
    for [x, y] in layers
        .iter()
        .flat_map(|layer| layer.strokes.iter())
        .chain(groups.iter().flatten())
        .flat_map(|stroke| [stroke[0], stroke[1]])
    {
        if x < inset || x > inner_x || y < inset || y > inner_y {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "drawing_sheet",
                "drawing content does not fit the sheet frame",
            ));
        }
    }
    // Nor may any content stroke cross the title block's interior: a view, its annotation or
    // a drafting object drawn over the block refuses like content beyond the frame (issue
    // 546). Segments are clipped against the block, since a stroke can cross it with both
    // ends outside; touching the block's border is allowed.
    let block_box = [[block_x, inset], [inner_x, inset + TITLE_BLOCK_HEIGHT_MM]];
    if layers
        .iter()
        .filter(|layer| !matches!(layer.class, "frame" | "titleblock"))
        .flat_map(|layer| layer.strokes.iter())
        .chain(groups.iter().flatten())
        .any(|stroke| crosses_interior(*stroke, block_box))
    {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "drawing_sheet",
            "drawing content overlaps the title block",
        ));
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
    Ok(SheetLayout {
        width_mm,
        height_mm,
        metadata,
        layers,
        primary: groups,
    })
}

/// Renders the exact resolved published geometry of every view as deterministic
/// stroke-only sheet SVG bytes, upright in SVG's Y-down space. The document must carry an
/// admitted sheet and fully resolved views; anything unresolved, out of the frame or over
/// the byte cap refuses. Every view is annotated below its geometry with its title and
/// display scale, and every drawn object on a visible layer renders on the sheet.
pub fn export_svg(
    drawing: &Drawing,
    version: DocumentVersionId,
    primary: &Projection,
    placed_bodies: &[BodyProjection],
    section_bodies: &[SectionProjection],
    detail_bodies: &[DetailProjection],
) -> Result<Vec<u8>, CommandError> {
    // A stored revision with a now-undrawable name, title or title block exports as blanks;
    // refuse the drawn surface honestly instead (visual audit finding 11).
    drawing.ensure_drawable_text()?;
    let layout = layout(
        drawing,
        version,
        primary,
        placed_bodies,
        section_bodies,
        detail_bodies,
    )?;
    let spec = drawing.spec();
    let mut text = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <svg xmlns=\"http://www.w3.org/2000/svg\" version=\"1.1\" width=\"{:.2}mm\" \
         height=\"{:.2}mm\" viewBox=\"0 0 {:.2} {:.2}\">\n\
         <title>{}</title>\n<metadata>{}</metadata>\n\
         <g fill=\"none\" stroke=\"#000000\" stroke-width=\"0.25\" stroke-linecap=\"round\">\n",
        layout.width_mm,
        layout.height_mm,
        layout.width_mm,
        layout.height_mm,
        escape(&spec.name),
        layout.metadata,
    );
    for layer in &layout.layers {
        text.push_str(&path_element(
            layer.class,
            layer_key(layer),
            layer.dash,
            &layer.strokes,
            layout.height_mm,
        ));
    }
    let [body, extension, line, arrows, label] = layout.primary;
    let top = layout.height_mm;
    text.push_str(&format!(
        "<path class=\"body\" d=\"{}\"/>\n<path class=\"extension\" d=\"{}\"/>\n\
         <path class=\"dimension\" d=\"{}\"/>\n<path class=\"arrow\" d=\"{}\"/>\n\
         <path class=\"label\" d=\"{}\"/>\n</g>\n</svg>\n",
        path(&flipped_about(body, top)),
        path(&flipped_about(extension, top)),
        path(&flipped_about(line, top)),
        path(&flipped_about(arrows, top)),
        path(&flipped_about(label, top)),
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

/// Renders the exact resolved published sheet as a deterministic stroke-only PDF on the
/// sheet's own paper: the same fixed sheet the SVG surface renders, with the frame, the
/// title block, every resolved view and every drawn object on a visible layer. Anything
/// unresolved, out of the frame or over the byte cap refuses.
pub fn export_pdf(
    drawing: &Drawing,
    version: DocumentVersionId,
    primary: &Projection,
    placed_bodies: &[BodyProjection],
    section_bodies: &[SectionProjection],
    detail_bodies: &[DetailProjection],
) -> Result<Vec<u8>, CommandError> {
    // A stored revision with a now-undrawable name, title or title block exports as blanks;
    // refuse the drawn surface honestly instead (visual audit finding 11).
    drawing.ensure_drawable_text()?;
    let layout = layout(
        drawing,
        version,
        primary,
        placed_bodies,
        section_bodies,
        detail_bodies,
    )?;
    let mut paths: Vec<PagePath> = layout
        .layers
        .iter()
        .map(|layer| PagePath {
            strokes: layer.strokes.clone(),
            dash_mm: layer.dash,
        })
        .collect();
    paths.extend(layout.primary.iter().map(|group| PagePath {
        strokes: group.clone(),
        dash_mm: None,
    }));
    pdf::write_page(
        layout.width_mm,
        layout.height_mm,
        &paths,
        [0.0, 0.0],
        &drawing.spec().name,
        &layout.metadata,
    )
}

/// Whether the segment passes through the open interior of the axis-aligned box `[min, max]`
/// (Liang–Barsky clipping): a segment that only touches the border does not count.
fn crosses_interior(segment: [[f64; 2]; 2], [min, max]: [[f64; 2]; 2]) -> bool {
    let [[x0, y0], [x1, y1]] = segment;
    let (dx, dy) = (x1 - x0, y1 - y0);
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    for (p, q) in [
        (-dx, x0 - min[0]),
        (dx, max[0] - x0),
        (-dy, y0 - min[1]),
        (dy, max[1] - y0),
    ] {
        if p == 0.0 {
            // Parallel to this edge: outside or on its line means no interior crossing.
            if q <= 0.0 {
                return false;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
        }
    }
    if t0 >= t1 {
        return false;
    }
    // The clipped piece lies inside the closed box; it crosses the interior unless it runs
    // along the border, which its midpoint decides.
    let t = 0.5 * (t0 + t1);
    let (mx, my) = (x0 + t * dx, y0 + t * dy);
    mx > min[0] && mx < max[0] && my > min[1] && my < max[1]
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
                paper_origin_mm: [30.0, 140.0],
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
                    paper_origin_mm: [60.0, 140.0],
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
                paper_mm: [[35.0, 145.0], [45.0, 145.0]],
            }]),
            dimension: DimensionProjection::Resolved(DimensionGeometry {
                anchors_paper_mm: [[35.0, 145.0], [45.0, 145.0]],
                line_paper_mm: [[35.0, 155.0], [45.0, 155.0]],
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
        let bodies = vec![placed_body([65.0, 145.0])];
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
    fn the_sheet_svg_renders_upright_not_mirrored() {
        // Finding 1 of the visual audit: the sheet writer emitted Y-up paper straight
        // into Y-down SVG, so the title block drew at the top and every glyph mirrored.
        // The frame's bottom edge must land at the largest SVG y, the top edge at the
        // smallest: A4 is 210 x 297 mm with a 10 mm inset.
        let drawing = sheet_drawing(Sheet::A4, &[D]);
        let version = DocumentVersionId::parse(C).unwrap();
        let bodies = vec![placed_body([65.0, 145.0])];
        let bytes = export_svg(&drawing, version, &projection(), &bodies, &[], &[]).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        let frame = text
            .split("<path class=\"frame\" d=\"")
            .nth(1)
            .unwrap_or_default();
        let frame = frame.split("/>").next().unwrap_or_default();
        assert!(
            frame.contains("M 10.0000 287.0000 L 200.0000 287.0000"),
            "{frame}"
        );
        // The frame's sides run into each other, so they continue one polyline instead of
        // restarting a subpath at every corner.
        assert!(
            frame.contains("L 200.0000 10.0000 L 10.0000 10.0000"),
            "{frame}"
        );
    }

    #[test]
    fn a_view_title_clears_the_lowest_stroke_of_its_view() {
        // Finding 5: the 3.0 mm gap is smaller than the 3.5 mm glyphs, so the title rose
        // 0.5 mm into the geometry. The title's tallest glyph must stay below the view's
        // lowest stroke: in flipped SVG space the title's closest point sits below the
        // geometry's own edge.
        let base = sheet_drawing(Sheet::A4, &[D]);
        let mut spec = base.spec().clone();
        spec.view.title = Some("ELEVATION".to_owned());
        let drawing = Drawing::admit(spec).unwrap();
        let version = DocumentVersionId::parse(C).unwrap();
        let bodies = vec![placed_body([65.0, 145.0])];
        let bytes = export_svg(&drawing, version, &projection(), &bodies, &[], &[]).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        let title = text
            .split("<path class=\"viewtitle\" data-view=\"B\" d=\"")
            .nth(1)
            .unwrap_or_default();
        let title = title.split("\"/>").next().unwrap_or_default();
        // The primary geometry's lowest stroke sits at paper y 45, flipped to svg y 252;
        // every title point must be below it.
        let numbers: Vec<f64> = title
            .split_whitespace()
            .filter_map(|token| token.parse::<f64>().ok())
            .collect();
        let highest_title_y = numbers
            .iter()
            .enumerate()
            .filter(|(index, _)| index % 2 == 1)
            .fold(f64::INFINITY, |min, (_, value)| min.min(*value));
        assert!(highest_title_y >= 252.0, "{title}");
    }

    #[test]
    fn the_title_block_closes_at_the_top_and_divides_after_the_label_column() {
        // Finding 6: the block was open at the top and its only vertical divider sat 30
        // mm from the frame's right edge, so labels and values shared one column and a
        // long project name struck through. The divider follows the label column and the
        // block closes at the top.
        let drawing = sheet_drawing(Sheet::A4, &[D]);
        let version = DocumentVersionId::parse(C).unwrap();
        let bodies = vec![placed_body([65.0, 145.0])];
        let bytes = export_svg(&drawing, version, &projection(), &bodies, &[], &[]).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        let block = text
            .split("<path class=\"titleblock\" d=\"")
            .nth(1)
            .unwrap_or_default();
        let block = block.split("\"/>").next().unwrap_or_default();
        // A4: inner_x 200, block_x 40; the block's top edge sits at paper y 54, flipped
        // to svg y 243.
        assert!(
            block.contains("M 40.0000 243.0000 L 200.0000 243.0000"),
            "{block}"
        );
        // The divider runs after the 30 mm label column: paper x 70, flipped top to
        // bottom of the block.
        assert!(
            block.contains("M 70.0000 287.0000 L 70.0000 243.0000"),
            "{block}"
        );
    }

    #[test]
    fn a_sheet_refuses_unsupported_inputs_instead_of_approximating() {
        let version = DocumentVersionId::parse(C).unwrap();
        let bodies = vec![placed_body([65.0, 145.0])];
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
                paper_mm: [[35.0, 145.0], [45.0, 145.0]],
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
            region_center_mm: [70.0, 145.0],
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
            paper_origin_mm: [90.0, 140.0],
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
            paper_origin_mm: [120.0, 140.0],
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
        let bodies = vec![placed_body([65.0, 145.0])];
        let sections = vec![section_body(
            vec![section_edge(95.0, 145.0)],
            vec![[95.0, 142.0], [105.0, 142.0], [105.0, 148.0]],
        )];
        let details = vec![detail_body(vec![section_edge(125.0, 145.0)])];
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

    /// Issue 546: a detail whose body clears the A4 title block (x 40..200, y 10..54) but
    /// whose annotation, 5 mm under its lowest stroke, lands inside the block refuses like
    /// content beyond the frame; lifted clear, the same detail exports.
    #[test]
    fn an_annotation_inside_the_title_block_refuses_instead_of_overlapping() {
        let version = DocumentVersionId::parse(C).unwrap();
        const DETAIL: &str = "01900000-0000-7000-8000-000000000343";
        let base = sheet_drawing(Sheet::A4, &[D]);
        let mut s = base.spec().clone();
        s.details = vec![detail_spec(DETAIL, B, Some("DET-1"))];
        let drawing = Drawing::admit(s).unwrap();
        let bodies = vec![placed_body([65.0, 145.0])];
        let low = vec![detail_body(vec![section_edge(125.0, 58.0)])];
        assert_eq!(
            export_svg(&drawing, version, &projection(), &bodies, &[], &low)
                .unwrap_err()
                .code,
            ErrorCode::LimitExceeded
        );
        assert_eq!(
            export_pdf(&drawing, version, &projection(), &bodies, &[], &low)
                .unwrap_err()
                .code,
            ErrorCode::LimitExceeded
        );
        let clear = vec![detail_body(vec![section_edge(125.0, 70.0)])];
        assert!(export_svg(&drawing, version, &projection(), &bodies, &[], &clear).is_ok());
    }

    #[test]
    fn the_primary_title_clears_the_dimension_hanging_under_the_view() {
        // Visual audit finding 9: the title cleared the body only, so a dimension under
        // the view boxed the title between its extension lines. The annotation now clears
        // the whole dimension group: extension lines, line, arrows and the dimension's
        // own label beneath the line.
        let base = sheet_drawing(Sheet::A4, &[]);
        let mut spec = base.spec().clone();
        spec.view.title = Some("ELEVATION".to_owned());
        let drawing = Drawing::admit(spec).unwrap();
        let version = DocumentVersionId::parse(C).unwrap();
        let mut projected = projection();
        if let DimensionProjection::Resolved(geometry) = &mut projected.dimension {
            // The dimension hangs under the body (paper y 145): the line at 135, its
            // label glyphs below the line around paper y 130.
            geometry.anchors_paper_mm = [[35.0, 145.0], [45.0, 145.0]];
            geometry.line_paper_mm = [[35.0, 135.0], [45.0, 135.0]];
        }
        let bytes = export_svg(&drawing, version, &projected, &[], &[], &[]).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        let title = text
            .split("<path class=\"viewtitle\" data-view=\"B\" d=\"")
            .nth(1)
            .unwrap_or_default();
        let title = title.split("\"/>").next().unwrap_or_default();
        // Paper y 130 is the dimension label's lowest reach; in flipped SVG space the
        // title must sit entirely below it.
        let numbers: Vec<f64> = title
            .split_whitespace()
            .filter_map(|token| token.parse::<f64>().ok())
            .collect();
        let highest_title_y = numbers
            .iter()
            .enumerate()
            .filter(|(index, _)| index % 2 == 1)
            .fold(f64::INFINITY, |min, (_, value)| min.min(*value));
        assert!(
            highest_title_y >= 297.0 - 130.0,
            "the title clears the dimension group: {title}"
        );
    }

    #[test]
    fn a_detail_draws_its_boundary_and_marks_its_region_on_the_parent() {
        // Visual audit finding 8: the detail drew a bare fragment with no region circle
        // on the parent and no boundary. The detail carries its region's ring, and the
        // sheet draws the callout circle on the parent at the detail's own region.
        let version = DocumentVersionId::parse(C).unwrap();
        const DETAIL: &str = "01900000-0000-7000-8000-000000000342";
        let base = sheet_drawing(Sheet::A4, &[D]);
        let mut s = base.spec().clone();
        s.details = vec![detail_spec(DETAIL, B, Some("DET-1"))];
        let drawing = Drawing::admit(s).unwrap();
        let bodies = vec![placed_body([65.0, 145.0])];
        let details = vec![detail_body(vec![section_edge(125.0, 145.0)])];
        let bytes = export_svg(&drawing, version, &projection(), &bodies, &[], &details).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        // The parent-side callout circle keyed to the detail.
        assert!(
            text.contains(&format!("class=\"detailmark\" data-view=\"{DETAIL}\"")),
            "{text}"
        );
        // The parent-side mark carries the detail's name beside the circle: more strokes
        // than the circle's own chords, and the name's glyphs drawn outside the circle.
        let mark = text
            .split(&format!(
                "<path class=\"detailmark\" data-view=\"{DETAIL}\" d=\""
            ))
            .nth(1)
            .unwrap_or_default();
        let mark = mark.split("\"/>").next().unwrap_or_default();
        let circle_only = circle_strokes([0.0, 0.0], 5.0)
            .map(|c| c.len())
            .unwrap_or(0);
        assert!(
            mark.matches(" L ").count() > circle_only,
            "the callout names the detail beside its circle: {mark}"
        );
        // The detail's own boundary ring: its body layer holds the mapped circle, at
        // least the 16-chord floor of the tessellation.
        let body = text
            .split(&format!("<path class=\"body\" data-view=\"{DETAIL}\" d=\""))
            .nth(1)
            .unwrap_or_default();
        let body = body.split("\"/>").next().unwrap_or_default();
        assert!(
            body.matches(" L ").count() >= 16,
            "the boundary ring is in the detail's body: {body}"
        );
    }

    #[test]
    fn two_details_of_one_region_stack_their_callouts() {
        // Two details of the same region wrote their names over each other on the parent:
        // the second callout steps up a line, so both stay readable.
        let version = DocumentVersionId::parse(C).unwrap();
        const FIRST: &str = "01900000-0000-7000-8000-000000000343";
        const SECOND: &str = "01900000-0000-7000-8000-000000000344";
        let base = sheet_drawing(Sheet::A4, &[D]);
        let mut s = base.spec().clone();
        let mut second = detail_spec(SECOND, B, Some("DET-2"));
        second.paper_origin_mm = [120.0, 60.0];
        s.details = vec![detail_spec(FIRST, B, Some("DET-1")), second];
        let drawing = Drawing::admit(s).unwrap();
        let bodies = vec![placed_body([65.0, 145.0])];
        let details = vec![
            detail_body(vec![section_edge(125.0, 145.0)]),
            detail_body(vec![section_edge(125.0, 65.0)]),
        ];
        let bytes = export_svg(&drawing, version, &projection(), &bodies, &[], &details).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        // Both marks share one circle; the second name sits a line further from it, so the
        // second mark's vertical extent is a line taller, whatever the SVG's y direction.
        let extent = |id: &str| -> f64 {
            let mark = text
                .split(&format!(
                    "<path class=\"detailmark\" data-view=\"{id}\" d=\""
                ))
                .nth(1)
                .unwrap_or_default();
            let mark = mark.split("\"/>").next().unwrap_or_default();
            let numbers: Vec<f64> = mark
                .split(|c: char| c == ' ' || c == ',' || c.is_ascii_alphabetic())
                .filter_map(|t| t.parse().ok())
                .collect();
            let ys: Vec<f64> = numbers
                .chunks(2)
                .filter_map(|p| p.get(1).copied())
                .collect();
            let high = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let low = ys.iter().copied().fold(f64::INFINITY, f64::min);
            high - low
        };
        assert!(
            extent(SECOND) - extent(FIRST) >= 3.5,
            "the two callouts sit on different lines: {} vs {}",
            extent(FIRST),
            extent(SECOND)
        );
    }

    #[test]
    fn only_a_crossing_of_the_open_box_counts_as_an_overlap() {
        let b = [[40.0, 10.0], [200.0, 54.0]];
        // Both ends outside, crossing the interior.
        assert!(crosses_interior([[30.0, 30.0], [210.0, 30.0]], b));
        // Entirely inside.
        assert!(crosses_interior([[50.0, 20.0], [60.0, 20.0]], b));
        // Along the top border, touching a corner, and fully outside.
        assert!(!crosses_interior([[40.0, 54.0], [200.0, 54.0]], b));
        assert!(!crosses_interior([[30.0, 64.0], [50.0, 54.0]], b));
        assert!(!crosses_interior([[30.0, 60.0], [210.0, 60.0]], b));
        // A diagonal clipping the corner region.
        assert!(crosses_interior([[30.0, 40.0], [50.0, 60.0]], b));
    }

    #[test]
    fn the_title_block_admits_only_what_the_stroke_glyphs_draw() {
        // Review round 2: the title block's project and author draw through the stroke
        // glyphs, so an admitted Cyrillic field would blank its own marks in the drawn
        // block. Both admission entry points refuse it.
        let fixture = sheet_drawing(Sheet::A4, &[]);
        assert!(Drawing::admit(fixture.spec().clone()).is_ok());
        let mut s = fixture.spec().clone();
        s.sheet.as_mut().unwrap().title.author = "Кузнецов".to_owned();
        assert_eq!(
            Drawing::admit(s).unwrap_err().field,
            "sheet_author",
            "the author refuses at admission"
        );
        let mut s = fixture.spec().clone();
        s.sheet.as_mut().unwrap().title.project = "Мост".to_owned();
        assert_eq!(
            Drawing::admit(s).unwrap_err().field,
            "sheet_project",
            "the project refuses at admission"
        );
        let mut sheet = sheet(Sheet::A4);
        sheet.title.project = "Мост".to_owned();
        assert_eq!(
            fixture.without_sheet().with_sheet(sheet).unwrap_err().field,
            "sheet_project",
            "with_sheet refuses like admit"
        );
    }
}
