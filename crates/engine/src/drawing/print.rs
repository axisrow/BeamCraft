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
//!
//! The page submits to the printer through CUPS `lp`, the native print service of macOS
//! and Linux desktops: the PDF travels on stdin, so a submission leaves no temporary file.
//! Any other platform refuses `BackendUnavailable` until a submission backend exists there.

use super::Drawing;
use super::pdf::{self, PagePath};
use super::projection::{ALGORITHM, Projection};
use super::svg::{bounds, dash_array, drafting_layers, stroke_groups};
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::DocumentVersionId;

pub use beamcraft_model::sheet::Sheet;

/// Version of the closed print pipeline contract.
pub const PRINT_CONTRACT: &str = "beamcraft-drawing-print/1";

/// Blank margin kept clear along every sheet edge, in millimetres.
pub const SHEET_MARGIN_MM: f64 = 10.0;

/// Lays the exact resolved published geometry out 1:1, centred on `sheet`, with every
/// drawn object on a visible layer, and renders the print-ready page through the PDF
/// writer. The output stays bound to `version`; the caller's model and document are never
/// touched. Content beyond the sheet margin refuses.
pub fn export(
    drawing: &Drawing,
    version: DocumentVersionId,
    projection: &Projection,
    sheet: Sheet,
) -> Result<Vec<u8>, CommandError> {
    let (groups, _) = stroke_groups(projection)?;
    let drafting = drafting_layers(drawing)?;
    let [min_x, min_y, max_x, max_y] = bounds(
        groups
            .iter()
            .flatten()
            .chain(drafting.iter().flat_map(|layer| layer.strokes.iter())),
    );
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
    let mut paths: Vec<PagePath> = groups
        .iter()
        .map(|group| PagePath {
            strokes: group.clone(),
            dash_mm: None,
        })
        .collect();
    paths.extend(drafting.iter().map(|layer| PagePath {
        strokes: layer.strokes.clone(),
        dash_mm: dash_array(layer.line_type),
    }));
    pdf::write_page(width_mm, height_mm, &paths, origin, &spec.name, &keywords)
}

/// One printer submission request for an exact rendered page: the PDF bytes, the job name
/// the queue shows, and the target queue (`None` is the system default).
pub struct PrintJob {
    /// The exact rendered page, in the shared PDF writer's byte form.
    pub pdf: Vec<u8>,
    /// The name the queue shows for this job; control characters are trimmed and the
    /// length is bounded at submission time.
    pub name: String,
    /// The target queue's name, or `None` for the system default printer.
    pub printer: Option<String>,
}

/// Longest queue name the submission admits, in bytes.
pub const PRINTER_NAME_MAX_BYTES: usize = 127;

/// Longest job name the submission admits, in bytes.
pub const JOB_NAME_MAX_BYTES: usize = 200;

/// What one submission produced: the queue's own request line.
#[derive(Debug)]
pub struct Submission {
    /// The request line `lp` printed for the accepted job.
    pub request: String,
}

/// Trims ASCII control characters and bounds `value` to `cap` bytes without splitting a
/// character; an empty remainder refuses.
fn bounded_text(value: &str, cap: usize, empty: &'static str) -> Result<String, CommandError> {
    let mut owned: String = value.chars().filter(|c| !c.is_ascii_control()).collect();
    while owned.len() > cap {
        owned.pop();
    }
    if owned.is_empty() {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "drawing_print",
            empty,
        ));
    }
    Ok(owned)
}

/// The exact `lp` invocation for `job`: one argument per entry, no shell. A queue name
/// must not start with `-` (it would read as an option) and stays within
/// [`PRINTER_NAME_MAX_BYTES`]; the job name is bounded to [`JOB_NAME_MAX_BYTES`]. Both
/// names keep no ASCII control characters.
pub fn lp_arguments(job: &PrintJob) -> Result<Vec<String>, CommandError> {
    let mut args = vec![String::from("lp")];
    if let Some(printer) = &job.printer {
        if printer.starts_with('-') {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "drawing_print",
                "a printer queue name may not start with '-'",
            ));
        }
        args.push(String::from("-d"));
        args.push(bounded_text(
            printer,
            PRINTER_NAME_MAX_BYTES,
            "the printer queue name is empty",
        )?);
    }
    args.push(String::from("-J"));
    args.push(bounded_text(
        &job.name,
        JOB_NAME_MAX_BYTES,
        "the print job name is empty",
    )?);
    Ok(args)
}

/// Submits the rendered page through CUPS `lp`: the PDF travels on stdin. A missing `lp`
/// or a rejecting queue is `BackendUnavailable`, never a panic, and nothing is written to
/// the filesystem.
#[cfg(unix)]
pub fn submit(job: &PrintJob) -> Result<Submission, CommandError> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let args = lp_arguments(job)?;
    let Some((program, rest)) = args.split_first() else {
        return Err(CommandError::new(
            ErrorCode::BackendUnavailable,
            "drawing_print",
            "the print invocation is empty",
        ));
    };
    let mut child = Command::new(program)
        .args(rest)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| {
            CommandError::new(
                ErrorCode::BackendUnavailable,
                "drawing_print",
                "the lp print command is not available",
            )
        })?;
    let Some(mut stdin) = child.stdin.take() else {
        return Err(CommandError::new(
            ErrorCode::BackendUnavailable,
            "drawing_print",
            "the print command closed its input",
        ));
    };
    if stdin.write_all(&job.pdf).is_err() {
        return Err(CommandError::new(
            ErrorCode::BackendUnavailable,
            "drawing_print",
            "the print command refused the page bytes",
        ));
    }
    drop(stdin);
    let output = child.wait_with_output().map_err(|_| {
        CommandError::new(
            ErrorCode::BackendUnavailable,
            "drawing_print",
            "the print command could not be waited on",
        )
    })?;
    if !output.status.success() {
        return Err(CommandError::new(
            ErrorCode::BackendUnavailable,
            "drawing_print",
            "the print command rejected the job",
        ));
    }
    Ok(Submission {
        request: String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_owned(),
    })
}

/// Any platform without a submission backend refuses honestly until one exists.
#[cfg(not(unix))]
pub fn submit(_job: &PrintJob) -> Result<Submission, CommandError> {
    Err(CommandError::new(
        ErrorCode::BackendUnavailable,
        "drawing_print",
        "printer submission has no backend on this platform yet",
    ))
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

    #[test]
    fn the_lp_invocation_is_plain_argv_with_bounded_clean_names() {
        let long: String = "m".repeat(500);
        let named = PrintJob {
            pdf: Vec::new(),
            name: format!("sheet\u{c}feed {long}"),
            printer: Some(String::from("shop-laser")),
        };
        let name = bounded_text(&named.name, JOB_NAME_MAX_BYTES, "empty").unwrap();
        assert!(!name.contains('\u{c}'));
        assert!(name.len() <= JOB_NAME_MAX_BYTES);
        assert_eq!(
            lp_arguments(&named).unwrap(),
            vec![
                String::from("lp"),
                String::from("-d"),
                String::from("shop-laser"),
                String::from("-J"),
                name,
            ]
        );
        let default_queue = PrintJob {
            pdf: Vec::new(),
            name: String::from("Member drawing"),
            printer: None,
        };
        assert_eq!(
            lp_arguments(&default_queue).unwrap(),
            vec![
                String::from("lp"),
                String::from("-J"),
                String::from("Member drawing"),
            ]
        );
        for bad in ["-evil", "\u{7}", ""] {
            let job = PrintJob {
                pdf: Vec::new(),
                name: String::from("x"),
                printer: Some(String::from(bad)),
            };
            assert_eq!(
                lp_arguments(&job).unwrap_err().code,
                ErrorCode::InvalidArgument,
                "{bad:?}"
            );
        }
        let empty_name = PrintJob {
            pdf: Vec::new(),
            name: String::from("\u{1}"),
            printer: None,
        };
        assert_eq!(
            lp_arguments(&empty_name).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_rejecting_queue_reports_the_backend_and_submits_nothing() {
        let job = PrintJob {
            pdf: b"%PDF-1.4\n".to_vec(),
            name: String::from("beamcraft print test"),
            printer: Some(String::from("beamcraft-no-such-queue")),
        };
        assert_eq!(
            submit(&job).unwrap_err().code,
            ErrorCode::BackendUnavailable
        );
    }

    #[cfg(not(unix))]
    #[test]
    fn a_platform_without_a_backend_refuses_the_submission() {
        let job = PrintJob {
            pdf: Vec::new(),
            name: String::from("x"),
            printer: None,
        };
        assert_eq!(
            submit(&job).unwrap_err().code,
            ErrorCode::BackendUnavailable
        );
    }
}
