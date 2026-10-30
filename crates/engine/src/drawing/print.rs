// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D08: the print pipeline shape for one exact published drawing version.
//!
//! The pipeline is: published version → strictly resolved strokes → fixed sheet layout →
//! deterministic PDF page. Layout places the geometry 1:1 in paper millimetres, centred on
//! the sheet. A drawing that cannot fit its sheet refuses instead of silently rescaling,
//! because a rescaled print would misstate every drawn size against its model-millimetre
//! label. The rendered page goes through the shared PDF writer, so fonts and symbols stay
//! BeamCraft's own stroke geometry.

use super::Drawing;
use super::pdf;
use super::projection::{ALGORITHM, Projection};
use super::svg::{bounds, stroke_groups};
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::DocumentVersionId;

/// Version of the closed print pipeline contract.
pub const PRINT_CONTRACT: &str = "beamcraft-drawing-print/1";

/// Blank margin kept clear along every sheet edge, in millimetres.
pub const SHEET_MARGIN_MM: f64 = 10.0;

/// Fixed international sheet sizes in portrait orientation, in increasing area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sheet {
    /// 210 × 297 mm.
    A4,
    /// 297 × 420 mm.
    A3,
    /// 420 × 594 mm.
    A2,
    /// 594 × 841 mm.
    A1,
    /// 841 × 1189 mm.
    A0,
}

impl Sheet {
    /// Portrait sheet size in millimetres, width then height.
    pub fn size_mm(self) -> [f64; 2] {
        match self {
            Self::A4 => [210.0, 297.0],
            Self::A3 => [297.0, 420.0],
            Self::A2 => [420.0, 594.0],
            Self::A1 => [594.0, 841.0],
            Self::A0 => [841.0, 1189.0],
        }
    }

    /// Exact lowercase wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::A4 => "a4",
            Self::A3 => "a3",
            Self::A2 => "a2",
            Self::A1 => "a1",
            Self::A0 => "a0",
        }
    }

    /// Parses a fixed sheet token.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "a4" => Some(Self::A4),
            "a3" => Some(Self::A3),
            "a2" => Some(Self::A2),
            "a1" => Some(Self::A1),
            "a0" => Some(Self::A0),
            _ => None,
        }
    }
}

/// Lays the exact resolved published geometry out 1:1, centred on `sheet`, and renders the
/// print-ready page through the PDF writer. The output stays bound to `version`; the
/// caller's model and document are never touched. Content beyond the sheet margin refuses.
pub fn export(
    drawing: &Drawing,
    version: DocumentVersionId,
    projection: &Projection,
    sheet: Sheet,
) -> Result<Vec<u8>, CommandError> {
    let (groups, _) = stroke_groups(projection)?;
    let [min_x, min_y, max_x, max_y] = bounds(&groups);
    let [width_mm, height_mm] = sheet.size_mm();
    if max_x - min_x > width_mm - 2.0 * SHEET_MARGIN_MM
        || max_y - min_y > height_mm - 2.0 * SHEET_MARGIN_MM
    {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "drawing_print",
            "the drawing does not fit the chosen sheet at 1:1",
        ));
    }
    let origin = [
        (max_x + min_x) / 2.0 - width_mm / 2.0,
        (max_y + min_y) / 2.0 - height_mm / 2.0,
    ];
    let spec = drawing.spec();
    let keywords = format!(
        "{} drawing={} view={} dimension={} version={version} sheet={} source_revision={} \
         source_manifest={} algorithm={}",
        PRINT_CONTRACT,
        spec.id,
        spec.view.id,
        spec.dimension.id,
        sheet.as_str(),
        spec.source.revision,
        spec.source.manifest,
        ALGORITHM,
    );
    pdf::write_page(width_mm, height_mm, &groups, origin, &spec.name, &keywords)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::drawing::projection::{
        BodyProjection, DimensionGeometry, DimensionProjection, ProjectedEdge, UnresolvedReason,
    };
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

    fn drawing() -> Drawing {
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
            views: Vec::new(),
        })
        .unwrap()
    }

    fn projection(span_mm: f64) -> Projection {
        Projection {
            body: BodyProjection::Resolved(vec![ProjectedEdge {
                feature: FeatureId::parse(&format!("fid1:{MEMBER}:member:e.x0.y0")).unwrap(),
                paper_mm: [[0.1, 0.05], [0.1 + span_mm, 0.05]],
            }]),
            dimension: DimensionProjection::Resolved(DimensionGeometry {
                anchors_paper_mm: [[0.1, 0.05], [0.1 + span_mm, 0.05]],
                line_paper_mm: [[0.1, 10.05], [0.1 + span_mm, 10.05]],
                projected_length_mm: span_mm * DEFAULT_SCALE_DENOMINATOR,
                label_mm: "1000".to_owned(),
            }),
        }
    }

    #[test]
    fn export_is_deterministic_centred_and_sheet_bound() {
        let drawing = drawing();
        let version = DocumentVersionId::parse(C).unwrap();
        let first = export(&drawing, version, &projection(10.0), Sheet::A4).unwrap();
        assert_eq!(
            first,
            export(&drawing, version, &projection(10.0), Sheet::A4).unwrap()
        );
        let text = std::str::from_utf8(&first).unwrap();
        assert!(text.starts_with("%PDF-1.4\n"));
        assert!(text.contains("/MediaBox [0 0 595.2756 841.8898]"));
        assert!(text.contains(&format!("version={version} sheet=a4")));
        assert!(!text.contains("/Font"));
        // Placement is translation invariant: the same content shifted on the paper
        // centres to identical page geometry.
        let mut shifted = projection(10.0);
        if let DimensionProjection::Resolved(geometry) = &mut shifted.dimension {
            for [x, y] in &mut geometry.anchors_paper_mm {
                *x += 40.0;
                *y += 25.0;
            }
            for [x, y] in &mut geometry.line_paper_mm {
                *x += 40.0;
                *y += 25.0;
            }
        }
        if let BodyProjection::Resolved(edges) = &mut shifted.body {
            for edge in edges {
                for [x, y] in &mut edge.paper_mm {
                    *x += 40.0;
                    *y += 25.0;
                }
            }
        }
        assert_eq!(
            export(&drawing, version, &shifted, Sheet::A4).unwrap(),
            first
        );
        // Every sheet parses and keeps its fixed portrait size.
        assert_eq!(Sheet::parse("a3"), Some(Sheet::A3));
        assert_eq!(Sheet::A4.size_mm(), [210.0, 297.0]);
        assert_eq!(Sheet::A0.size_mm(), [841.0, 1189.0]);
    }

    #[test]
    fn content_that_does_not_fit_refuses_instead_of_rescaling() {
        let drawing = drawing();
        let version = DocumentVersionId::parse(C).unwrap();
        // 400 mm of content exceeds the A4 print margin but fits A1 at 1:1.
        assert_eq!(
            export(&drawing, version, &projection(400.0), Sheet::A4)
                .unwrap_err()
                .code,
            ErrorCode::LimitExceeded
        );
        export(&drawing, version, &projection(400.0), Sheet::A1).unwrap();
        let unresolved = Projection {
            body: BodyProjection::Unresolved(UnresolvedReason::MissingOwner),
            dimension: DimensionProjection::Unresolved(UnresolvedReason::MissingOwner),
        };
        assert_eq!(
            export(&drawing, version, &unresolved, Sheet::A4)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
    }
}
