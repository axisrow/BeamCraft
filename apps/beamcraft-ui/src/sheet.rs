// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D05 sheet editor: hand placement of a drawing document's views on its sheet.
//!
//! The editor reads the document through the same `inspect-drawing` command a user runs,
//! shows the sheet furniture and one marker per placed view, section and detail, and
//! applies moved origins through the same `edit-views` command with fresh identities from
//! the shared [`IdSource`]. The UI computes no engineering geometry: the reproducible sheet
//! output is the engine's whole-sheet export, reached through the CLI. Dropping a marker
//! within the align tolerance of another view's axis snaps it onto that axis, so aligned
//! views are one gesture, not a numeric edit.

use crate::command::{self, Output};
use crate::stats::Stats;
use crate::tools::{ToolError, decode, rejection, text};
use beamcraft_engine::drawing::sheet::{TITLE_BLOCK_HEIGHT_MM, TITLE_BLOCK_WIDTH_MM};
use beamcraft_host::ids::IdSource;
use beamcraft_json::Json;
use beamcraft_model::sheet::Sheet;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::Receiver;

/// Two origins within this many millimetres count as aligned, and a dropped marker within
/// this distance of another view's axis snaps onto it.
const ALIGN_TOLERANCE_MM: f64 = 0.5;
/// Marker hit radius on screen.
const HIT_PX: f32 = 12.0;
/// The sheet sizes the command admits, in menu order.
const SIZES: [&str; 5] = ["a4", "a3", "a2", "a1", "a0"];

/// One placed or primary view as the document inspect reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    /// Stable view identity.
    pub id: String,
    /// Projection basis token (`front`, `top` or `right`).
    pub plane: String,
    /// Paper/model scale denominator.
    pub scale_denominator: f64,
    /// Paper-space origin in millimetres.
    pub origin: [f64; 2],
    /// The authored annotation beside the display scale; absent is the unannotated default.
    pub title: Option<String>,
}

/// One placed section view as the document inspect reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct SectionPlacement {
    /// Stable view identity.
    pub id: String,
    /// Projection basis token (`front`, `top` or `right`).
    pub plane: String,
    /// Cut position along the basis's depth axis, model millimetres.
    pub cut_mm: f64,
    /// Which side of the cut plane is visible (`positive` or `negative`).
    pub side: String,
    /// Paper/model scale denominator.
    pub scale_denominator: f64,
    /// Paper-space origin in millimetres.
    pub origin: [f64; 2],
    /// The authored annotation beside the display scale; absent is the unannotated default.
    pub title: Option<String>,
}

/// One placed detail view as the document inspect reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct DetailPlacement {
    /// Stable view identity.
    pub id: String,
    /// The parent view's id (the primary or a placed view).
    pub parent: String,
    /// The magnified region centre in the parent's paper millimetres.
    pub center: [f64; 2],
    /// The magnified region radius in parent paper millimetres.
    pub radius_mm: f64,
    /// Paper/model scale denominator.
    pub scale_denominator: f64,
    /// Paper-space origin in millimetres.
    pub origin: [f64; 2],
    /// The authored annotation beside the display scale; absent is the unannotated default.
    pub title: Option<String>,
}

/// The sheet the document is laid on, mirrored for the preview.
#[derive(Debug, Clone, PartialEq)]
pub struct Furniture {
    /// Sheet size token (`a4` through `a0`).
    pub size: String,
    /// Portrait sheet size in millimetres, width then height.
    pub sheet_mm: [f64; 2],
    /// Frame inset from every sheet edge.
    pub frame_inset_mm: f64,
    /// Title block project field.
    pub project: String,
    /// Title block author field.
    pub author: String,
}

/// One reading of the document.
#[derive(Debug, Clone, PartialEq)]
pub struct Doc {
    /// The revision every edit names as its expected head.
    pub revision_id: String,
    /// The drawing name.
    pub name: String,
    /// The sheet furniture, present only on a sheet document.
    pub furniture: Option<Furniture>,
    /// The primary view; its placement is document intent, not editor state.
    pub primary: Placement,
    /// The placed views in document order.
    pub placed: Vec<Placement>,
    /// The placed sections in document order.
    pub sections: Vec<SectionPlacement>,
    /// The placed details in document order.
    pub details: Vec<DetailPlacement>,
}

fn coordinate(json: &Json, key: &str, index: usize) -> Option<f64> {
    json.get(key).ok()?.array().ok()?.get(index)?.float().ok()
}

/// One view's optional title: absent or null is the unannotated default; anything else must
/// be a string or the whole reply is malformed for this editor.
fn title_of(json: &Json) -> Option<Option<String>> {
    match json.get("title") {
        Err(_) | Ok(Json::Null) => Some(None),
        Ok(value) => value.string().ok().map(|title| Some(title.to_owned())),
    }
}

fn placement(json: &Json) -> Option<Placement> {
    Some(Placement {
        id: text(json, "id")?,
        plane: text(json, "plane")?,
        scale_denominator: json.get("scale_denominator").ok()?.float().ok()?,
        origin: [
            coordinate(json, "paper_origin_mm", 0)?,
            coordinate(json, "paper_origin_mm", 1)?,
        ],
        title: title_of(json)?,
    })
}

fn section(json: &Json) -> Option<SectionPlacement> {
    Some(SectionPlacement {
        id: text(json, "id")?,
        plane: text(json, "plane")?,
        cut_mm: json.get("cut_mm").ok()?.float().ok()?,
        side: text(json, "side")?,
        scale_denominator: json.get("scale_denominator").ok()?.float().ok()?,
        origin: [
            coordinate(json, "paper_origin_mm", 0)?,
            coordinate(json, "paper_origin_mm", 1)?,
        ],
        title: title_of(json)?,
    })
}

fn detail(json: &Json) -> Option<DetailPlacement> {
    Some(DetailPlacement {
        id: text(json, "id")?,
        parent: text(json, "parent")?,
        center: [
            coordinate(json, "center_paper_mm", 0)?,
            coordinate(json, "center_paper_mm", 1)?,
        ],
        radius_mm: json.get("radius_mm").ok()?.float().ok()?,
        scale_denominator: json.get("scale_denominator").ok()?.float().ok()?,
        origin: [
            coordinate(json, "paper_origin_mm", 0)?,
            coordinate(json, "paper_origin_mm", 1)?,
        ],
        title: title_of(json)?,
    })
}

/// The origins of every movable view, flat in document order: placed views, then sections,
/// then details. `pending` mirrors this order one to one.
fn movable(doc: &Doc) -> Vec<[f64; 2]> {
    doc.placed
        .iter()
        .map(|view| view.origin)
        .chain(doc.sections.iter().map(|section| section.origin))
        .chain(doc.details.iter().map(|detail| detail.origin))
        .collect()
}

/// One view's optional title segment of a spec value: `Some` is the whole remainder (it may
/// hold colons), `None` ends the value after the origin.
fn title_segment(title: &Option<String>) -> String {
    match title {
        Some(title) => format!(":{title}"),
        None => String::new(),
    }
}

/// The `edit-views` option values that put every moved origin back: one `--placed-view`,
/// `--placed-section` and `--placed-detail` value per placed view, section and detail, each
/// carrying the kind's whole state. edit-views replaces the three sets together, so sending
/// only the moved family would delete the other two.
fn placement_args(doc: &Doc, pending: &[[f64; 2]]) -> Vec<String> {
    let origins = movable(doc);
    let origin_of = |index: usize| {
        pending
            .get(index)
            .or_else(|| origins.get(index))
            .copied()
            .unwrap_or_default()
    };
    let mut args = Vec::new();
    for (index, view) in doc.placed.iter().enumerate() {
        args.push("--placed-view".to_owned());
        args.push(format!(
            "{}:{}:{}:{:.2},{:.2}{}",
            view.id,
            view.plane,
            fmt_number(view.scale_denominator),
            origin_of(index)[0],
            origin_of(index)[1],
            title_segment(&view.title),
        ));
    }
    let sections_at = doc.placed.len();
    for (index, section) in doc.sections.iter().enumerate() {
        args.push("--placed-section".to_owned());
        args.push(format!(
            "{}:{}:{}:{}:{}:{:.2},{:.2}{}",
            section.id,
            section.plane,
            fmt_number(section.cut_mm),
            section.side,
            fmt_number(section.scale_denominator),
            origin_of(sections_at + index)[0],
            origin_of(sections_at + index)[1],
            title_segment(&section.title),
        ));
    }
    let details_at = sections_at + doc.sections.len();
    for (index, detail) in doc.details.iter().enumerate() {
        args.push("--placed-detail".to_owned());
        args.push(format!(
            "{}:{}:{:.2},{:.2}:{}:{}:{:.2},{:.2}{}",
            detail.id,
            detail.parent,
            detail.center[0],
            detail.center[1],
            fmt_number(detail.radius_mm),
            fmt_number(detail.scale_denominator),
            origin_of(details_at + index)[0],
            origin_of(details_at + index)[1],
            title_segment(&detail.title),
        ));
    }
    args
}

/// The full `edit-views` command that applies `pending` to `doc`: every placed view, section
/// and detail goes back, moved or not, with fresh identities. Engine admission still owns
/// every bound; this only builds the command the editor itself spawns.
pub fn apply_args(
    document_dir: &str,
    expected_head: &str,
    op_id: &str,
    revision_id: &str,
    actor_id: &str,
    doc: &Doc,
    pending: &[[f64; 2]],
) -> Vec<String> {
    let mut args = vec![
        "edit-views".to_owned(),
        "--document-dir".to_owned(),
        document_dir.to_owned(),
        "--expected-head".to_owned(),
        expected_head.to_owned(),
        "--op-id".to_owned(),
        op_id.to_owned(),
        "--revision-id".to_owned(),
        revision_id.to_owned(),
        "--actor-id".to_owned(),
        actor_id.to_owned(),
    ];
    args.extend(placement_args(doc, pending));
    args
}

/// Reads the `inspect-drawing` reply into the editor's document mirror. `None` means the
/// reply is malformed for this editor; nothing is guessed from partial fields.
pub fn parse_doc(reply: &Json) -> Option<Doc> {
    let furniture = match reply.get("sheet") {
        Ok(Json::Null) | Err(_) => None,
        Ok(sheet) => {
            let token = text(sheet, "size")?;
            let [width, height] = Sheet::parse(&token)?.size_mm();
            Some(Furniture {
                size: token,
                sheet_mm: [width, height],
                frame_inset_mm: sheet.get("frame_inset_mm").ok()?.float().ok()?,
                project: text(sheet, "project")?,
                author: text(sheet, "author")?,
            })
        }
    };
    Some(Doc {
        revision_id: text(reply, "revision_id")?,
        name: text(reply, "name")?,
        furniture,
        primary: placement(reply.get("view").ok()?)?,
        placed: reply
            .get("placed_views")
            .ok()?
            .array()
            .ok()?
            .iter()
            .map(placement)
            .collect::<Option<Vec<_>>>()?,
        sections: reply
            .get("placed_sections")
            .ok()?
            .array()
            .ok()?
            .iter()
            .map(section)
            .collect::<Option<Vec<_>>>()?,
        details: reply
            .get("placed_details")
            .ok()?
            .array()
            .ok()?
            .iter()
            .map(detail)
            .collect::<Option<Vec<_>>>()?,
    })
}

/// Keeps a moved origin inside the frame interior and on the tenth-millimetre grid.
pub fn constrain(origin: [f64; 2], sheet_mm: [f64; 2], inset_mm: f64) -> [f64; 2] {
    let step = |value: f64, low: f64, high: f64| ((value * 10.0).round() / 10.0).clamp(low, high);
    [
        step(origin[0], inset_mm, sheet_mm[0] - inset_mm),
        step(origin[1], inset_mm, sheet_mm[1] - inset_mm),
    ]
}

/// Snaps a dropped origin onto any other view's aligned axis within the tolerance.
pub fn snap(origin: [f64; 2], others: &[[f64; 2]]) -> [f64; 2] {
    let mut snapped = origin;
    for other in others {
        if (snapped[0] - other[0]).abs() <= ALIGN_TOLERANCE_MM {
            snapped[0] = other[0];
        }
        if (snapped[1] - other[1]).abs() <= ALIGN_TOLERANCE_MM {
            snapped[1] = other[1];
        }
    }
    snapped
}

/// What a finished background command was doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Job {
    /// `inspect-drawing`: replace the whole reading.
    Load,
    /// `edit-views`: placements were applied; reload afterwards.
    Apply,
    /// `edit-sheet`: the sheet was laid on or taken off; reload afterwards.
    EditSheet {
        /// Whether the edit took the sheet off.
        remove: bool,
    },
}

/// The typed sheet form: a size with its title-block fields and the frame inset; an
/// empty inset is the command's own default.
pub struct LaySheet<'a> {
    /// The sheet size token (`a4` through `a0`).
    pub size: &'a str,
    /// The title-block project field.
    pub project: &'a str,
    /// The title-block author field.
    pub author: &'a str,
    /// The frame inset in millimetres; `None` lets the command default it.
    pub frame_inset_mm: Option<f64>,
}

/// The typed frame inset: empty is the command's own default, a finite number passes
/// through, anything else is a local refusal whose message the window shows.
pub fn parse_frame(field: &str) -> Result<Option<f64>, String> {
    let text = field.trim();
    if text.is_empty() {
        return Ok(None);
    }
    match text.parse::<f64>() {
        Ok(value) if value.is_finite() => Ok(Some(value)),
        _ => Err("invalid sheet frame inset".to_owned()),
    }
}

/// The `edit-sheet` arguments that lay the typed sheet on the document. Engine admission
/// still owns every bound; this only builds the command.
pub fn sheet_args(
    document_dir: &str,
    expected_head: &str,
    op_id: &str,
    revision_id: &str,
    actor_id: &str,
    sheet: &LaySheet<'_>,
) -> Vec<String> {
    let mut args = vec![
        "edit-sheet".to_owned(),
        "--document-dir".to_owned(),
        document_dir.to_owned(),
        "--expected-head".to_owned(),
        expected_head.to_owned(),
        "--op-id".to_owned(),
        op_id.to_owned(),
        "--revision-id".to_owned(),
        revision_id.to_owned(),
        "--actor-id".to_owned(),
        actor_id.to_owned(),
        "--sheet-size".to_owned(),
        sheet.size.to_owned(),
        "--sheet-project".to_owned(),
        sheet.project.to_owned(),
        "--sheet-author".to_owned(),
        sheet.author.to_owned(),
    ];
    if let Some(frame) = sheet.frame_inset_mm {
        args.extend(["--sheet-frame".to_owned(), format!("{frame}")]);
    }
    args
}

/// The `edit-sheet` arguments that take the sheet off the document.
pub fn remove_sheet_args(
    document_dir: &str,
    expected_head: &str,
    op_id: &str,
    revision_id: &str,
    actor_id: &str,
) -> Vec<String> {
    vec![
        "edit-sheet".to_owned(),
        "--document-dir".to_owned(),
        document_dir.to_owned(),
        "--expected-head".to_owned(),
        expected_head.to_owned(),
        "--op-id".to_owned(),
        op_id.to_owned(),
        "--revision-id".to_owned(),
        revision_id.to_owned(),
        "--actor-id".to_owned(),
        actor_id.to_owned(),
        "--sheet-remove".to_owned(),
        "true".to_owned(),
    ]
}

/// The sheet editor window state.
pub struct SheetEditor {
    /// Whether the window is shown.
    pub open: bool,
    /// The document directory text field.
    pub document_dir: String,
    doc: Option<Doc>,
    /// Pending origins of `doc.placed`, parallel by index.
    pending: Vec<[f64; 2]>,
    note: Option<Result<String, String>>,
    job: Option<(Job, Receiver<Result<Output, ToolError>>)>,
    drag: Option<(usize, [f64; 2], [f64; 2])>,
    /// The sheet form's size choice (`a4` through `a0`), starting on the command's own
    /// print default: an empty selection would refuse at the CLI after the title-block
    /// fields are typed.
    sheet_size: String,
    /// The sheet form's title-block fields as typed.
    sheet_project: String,
    /// The sheet form's author field as typed.
    sheet_author: String,
    /// The sheet form's frame inset as typed; empty is the command's own default.
    sheet_frame: String,
}

impl Default for SheetEditor {
    fn default() -> Self {
        Self {
            open: false,
            document_dir: String::new(),
            doc: None,
            pending: Vec::new(),
            note: None,
            job: None,
            drag: None,
            sheet_size: "a4".to_owned(),
            sheet_project: String::new(),
            sheet_author: String::new(),
            sheet_frame: String::new(),
        }
    }
}

impl SheetEditor {
    /// Shows the window and advances its background command, when open.
    pub fn window(
        &mut self,
        ctx: &egui::Context,
        cli: &Path,
        actor: Option<&str>,
        stats: &Arc<Stats>,
    ) {
        if !self.open {
            return;
        }
        self.poll(cli, stats);
        let mut open = self.open;
        crate::kit::tool_window("Sheet editor", &mut open, ctx)
            .resizable(true)
            .show(ctx, |ui| self.body(ui, cli, actor, stats));
        self.open = open;
    }

    fn body(&mut self, ui: &mut egui::Ui, cli: &Path, actor: Option<&str>, stats: &Arc<Stats>) {
        ui.horizontal(|ui| {
            ui.label("Document");
            ui.add(
                egui::TextEdit::singleline(&mut self.document_dir)
                    .desired_width(280.0)
                    .hint_text("path to the document directory"),
            );
            if ui
                .add_enabled(
                    self.job.is_none() && !self.document_dir.is_empty(),
                    egui::Button::new("Load"),
                )
                .clicked()
            {
                self.note = None;
                self.start(
                    cli,
                    stats,
                    Job::Load,
                    vec![
                        "inspect-drawing".to_owned(),
                        "--document-dir".to_owned(),
                        self.document_dir.clone(),
                    ],
                );
            }
        });
        let Some(doc) = self.doc.clone() else {
            ui.label("Load a drawing document to see its sheet and move the placed views by hand.");
            self.note_view(ui);
            return;
        };
        let Some(furniture) = &doc.furniture else {
            ui.label("This document carries no sheet. Lay one on to see it here.");
            self.sheet_form(ui, cli, actor, stats, &doc);
            self.note_view(ui);
            return;
        };
        ui.separator();
        ui.label(format!(
            "{} — sheet {}, frame {} mm",
            doc.name, furniture.size, furniture.frame_inset_mm
        ));
        self.preview(ui, &doc, furniture);
        let origins = movable(&doc);
        let moved = self
            .pending
            .iter()
            .zip(&origins)
            .any(|(pending, origin)| pending != origin);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    moved && self.job.is_none() && !origins.is_empty(),
                    egui::Button::new("Apply placements"),
                )
                .clicked()
            {
                self.note = None;
                self.apply(cli, actor, stats, &doc);
            }
            if ui.add_enabled(moved, egui::Button::new("Reset")).clicked() {
                self.pending = origins.clone();
            }
        });
        ui.label(format!(
            "{} placed views, {} sections and {} details; the primary view is placed by the document, not here.",
            doc.placed.len(),
            doc.sections.len(),
            doc.details.len()
        ));
        crate::kit::section(ui, "sheet", "Sheet", false, |ui| {
            self.sheet_form(ui, cli, actor, stats, &doc)
        });
        self.note_view(ui);
    }

    /// The `edit-sheet` form: a size with its title-block fields and the frame inset,
    /// laid on or taken off through the same command a user types.
    fn sheet_form(
        &mut self,
        ui: &mut egui::Ui,
        cli: &Path,
        actor: Option<&str>,
        stats: &Arc<Stats>,
        doc: &Doc,
    ) {
        ui.horizontal(|ui| {
            ui.label("Size");
            egui::ComboBox::from_id_salt("sheet-size")
                .selected_text(self.sheet_size.clone())
                .show_ui(ui, |ui| {
                    for size in SIZES {
                        ui.selectable_value(&mut self.sheet_size, size.to_owned(), size);
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("Project");
            ui.add(
                egui::TextEdit::singleline(&mut self.sheet_project)
                    .desired_width(200.0)
                    .hint_text("title-block project"),
            );
            ui.label("Author");
            ui.add(
                egui::TextEdit::singleline(&mut self.sheet_author)
                    .desired_width(160.0)
                    .hint_text("title-block author"),
            );
        });
        ui.horizontal(|ui| {
            ui.label("Frame inset mm");
            ui.add(
                egui::TextEdit::singleline(&mut self.sheet_frame)
                    .desired_width(60.0)
                    .hint_text("10"),
            );
            let ready =
                !self.sheet_project.trim().is_empty() && !self.sheet_author.trim().is_empty();
            if ui
                .add_enabled(
                    ready && self.job.is_none(),
                    egui::Button::new("Apply sheet"),
                )
                .clicked()
            {
                self.note = None;
                self.apply_sheet(cli, actor, stats, doc, false);
            }
            if ui
                .add_enabled(self.job.is_none(), egui::Button::new("Remove sheet"))
                .clicked()
            {
                self.note = None;
                self.apply_sheet(cli, actor, stats, doc, true);
            }
        });
    }

    /// Lays the typed sheet on the document or takes the sheet off, through `edit-sheet`
    /// with fresh identities; the reload afterwards re-earns the document head.
    fn apply_sheet(
        &mut self,
        cli: &Path,
        actor: Option<&str>,
        stats: &Arc<Stats>,
        doc: &Doc,
        remove: bool,
    ) {
        let missing = || "no identity could be generated".to_owned();
        let Some(op) = fresh_id() else {
            self.note = Some(Err(missing()));
            return;
        };
        let Some(revision) = fresh_id() else {
            self.note = Some(Err(missing()));
            return;
        };
        let actor = actor
            .map(str::to_owned)
            .or_else(fresh_id)
            .unwrap_or_default();
        let args = if remove {
            remove_sheet_args(&self.document_dir, &doc.revision_id, &op, &revision, &actor)
        } else {
            let frame = match parse_frame(&self.sheet_frame) {
                Ok(frame) => frame,
                Err(message) => {
                    self.note = Some(Err(message));
                    return;
                }
            };
            sheet_args(
                &self.document_dir,
                &doc.revision_id,
                &op,
                &revision,
                &actor,
                &LaySheet {
                    size: &self.sheet_size,
                    project: &self.sheet_project,
                    author: &self.sheet_author,
                    frame_inset_mm: frame,
                },
            )
        };
        self.start(cli, stats, Job::EditSheet { remove }, args);
    }

    fn note_view(&mut self, ui: &mut egui::Ui) {
        match &self.note {
            Some(Ok(message)) => {
                ui.label(message);
            }
            Some(Err(message)) => {
                ui.colored_label(egui::Color32::from_rgb(230, 80, 60), message);
            }
            None => {}
        }
    }

    /// The sheet preview: paper, frame, title block and one marker per view.
    fn preview(&mut self, ui: &mut egui::Ui, doc: &Doc, furniture: &Furniture) {
        let [width_mm, height_mm] = furniture.sheet_mm;
        let available = ui.available_size();
        let scale = (available.x / width_mm as f32)
            .min(available.y / height_mm as f32)
            .clamp(0.5, 2.5);
        let size = egui::vec2(width_mm as f32 * scale, height_mm as f32 * scale);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        let painter = ui.painter();
        let to_screen = |p: [f64; 2]| {
            egui::pos2(
                rect.left() + (p[0] as f32) * scale,
                rect.bottom() - (p[1] as f32) * scale,
            )
        };
        let to_paper = |pos: egui::Pos2| -> [f64; 2] {
            [
                ((pos.x - rect.left()) / scale) as f64,
                ((rect.bottom() - pos.y) / scale) as f64,
            ]
        };
        let frame = egui::Rect::from_min_max(
            to_screen([
                furniture.frame_inset_mm,
                height_mm - furniture.frame_inset_mm,
            ]),
            to_screen([
                width_mm - furniture.frame_inset_mm,
                furniture.frame_inset_mm,
            ]),
        );
        painter.rect_filled(rect, 0, egui::Color32::from_gray(250));
        painter.rect_stroke(
            frame,
            0,
            egui::Stroke::new(1.0, egui::Color32::BLACK),
            egui::StrokeKind::Inside,
        );
        // Title block: a plain box with the two authored fields, bottom right; the engine's
        // own block size, so the preview shows the same block the export draws and enforces.
        let block = egui::Rect::from_min_max(
            egui::pos2(
                frame.right() - TITLE_BLOCK_WIDTH_MM as f32 * scale,
                frame.bottom() - TITLE_BLOCK_HEIGHT_MM as f32 * scale,
            ),
            egui::pos2(frame.right(), frame.bottom()),
        );
        painter.rect_stroke(
            block,
            0,
            egui::Stroke::new(0.75, egui::Color32::BLACK),
            egui::StrokeKind::Inside,
        );
        painter.text(
            block.left_top() + egui::vec2(4.0, 10.0),
            egui::Align2::LEFT_TOP,
            format!("PROJECT  {}", furniture.project),
            egui::FontId::proportional(10.0),
            egui::Color32::BLACK,
        );
        painter.text(
            block.left_top() + egui::vec2(4.0, 24.0),
            egui::Align2::LEFT_TOP,
            format!("AUTHOR  {}", furniture.author),
            egui::FontId::proportional(10.0),
            egui::Color32::BLACK,
        );
        // Align guides while a marker is dragged.
        let origins = movable(doc);
        if let Some((index, ..)) = &self.drag
            && let Some(origin) = self.pending.get(*index)
        {
            for other in origins
                .iter()
                .enumerate()
                .filter(|(other, _)| other != index)
                .map(|(_, origin)| *origin)
                .chain(std::iter::once(doc.primary.origin))
            {
                if (origin[0] - other[0]).abs() <= ALIGN_TOLERANCE_MM {
                    painter.line_segment(
                        [to_screen([other[0], 0.0]), to_screen([other[0], height_mm])],
                        egui::Stroke::new(0.5, egui::Color32::from_rgb(60, 120, 230)),
                    );
                }
                if (origin[1] - other[1]).abs() <= ALIGN_TOLERANCE_MM {
                    painter.line_segment(
                        [to_screen([0.0, other[1]]), to_screen([width_mm, other[1]])],
                        egui::Stroke::new(0.5, egui::Color32::from_rgb(60, 120, 230)),
                    );
                }
            }
        }
        let marker = |painter: &egui::Painter, origin: [f64; 2], label: String, color| {
            let pos = to_screen(origin);
            painter.circle_filled(pos, 4.0, color);
            painter.text(
                pos + egui::vec2(6.0, -6.0),
                egui::Align2::LEFT_BOTTOM,
                label,
                egui::FontId::proportional(10.0),
                color,
            );
        };
        marker(
            painter,
            doc.primary.origin,
            format!(
                "primary {} 1:{}",
                doc.primary.plane,
                fmt_number(doc.primary.scale_denominator)
            ),
            egui::Color32::from_gray(120),
        );
        // One marker per movable view, flat in document order: views, sections, details.
        let mut labels = Vec::new();
        for view in &doc.placed {
            labels.push(format!(
                "{} 1:{}",
                view.plane,
                fmt_number(view.scale_denominator)
            ));
        }
        for section in &doc.sections {
            labels.push(format!(
                "{} 1:{}",
                section.plane,
                fmt_number(section.scale_denominator)
            ));
        }
        for detail in &doc.details {
            labels.push(format!("1:{}", fmt_number(detail.scale_denominator)));
        }
        for ((index, origin), label) in origins.iter().enumerate().zip(&labels) {
            let origin = self.pending.get(index).copied().unwrap_or(*origin);
            marker(
                painter,
                origin,
                label.clone(),
                egui::Color32::from_rgb(20, 90, 200),
            );
        }
        // Drag a placed marker: grab the nearest one under the pointer.
        if let Some(pointer) = response.interact_pointer_pos() {
            let paper = to_paper(pointer);
            if response.drag_started() {
                let hit = origins.iter().enumerate().find(|(index, origin)| {
                    let origin = self.pending.get(*index).copied().unwrap_or(**origin);
                    let screen = to_screen(origin);
                    ((screen - pointer).length() as f64) <= HIT_PX as f64
                });
                if let Some((index, origin)) = hit {
                    self.drag = Some((index, paper, *origin));
                }
            } else if let Some((index, grab, start)) = &self.drag {
                let moved = [start[0] + paper[0] - grab[0], start[1] + paper[1] - grab[1]];
                let constrained = constrain(moved, furniture.sheet_mm, furniture.frame_inset_mm);
                if let Some(slot) = self.pending.get_mut(*index) {
                    *slot = constrained;
                }
            }
        }
        if response.drag_stopped()
            && let Some((index, ..)) = self.drag.take()
        {
            let others = origins
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != index)
                .map(|(_, origin)| *origin)
                .chain(std::iter::once(doc.primary.origin))
                .collect::<Vec<_>>();
            if let Some(origin) = self.pending.get(index) {
                let snapped = snap(*origin, &others);
                if let Some(slot) = self.pending.get_mut(index) {
                    *slot = snapped;
                }
            }
        }
    }

    /// Applies every pending origin through `edit-views` with fresh identities. The command
    /// replaces the placed views, sections and details together, so every one of them goes
    /// back, moved or not.
    fn apply(&mut self, cli: &Path, actor: Option<&str>, stats: &Arc<Stats>, doc: &Doc) {
        let missing = || "no identity could be generated".to_owned();
        let Some(op) = fresh_id() else {
            self.note = Some(Err(missing()));
            return;
        };
        let Some(revision) = fresh_id() else {
            self.note = Some(Err(missing()));
            return;
        };
        let actor = actor
            .map(str::to_owned)
            .or_else(fresh_id)
            .unwrap_or_default();
        let args = apply_args(
            &self.document_dir,
            &doc.revision_id,
            &op,
            &revision,
            &actor,
            doc,
            &self.pending,
        );
        self.start(cli, stats, Job::Apply, args);
    }

    /// Spawns one CLI command on a worker thread.
    fn start(&mut self, cli: &Path, stats: &Arc<Stats>, job: Job, args: Vec<String>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let cli = cli.to_path_buf();
        let stats = Arc::clone(stats);
        let spawned = std::thread::Builder::new()
            .name("beamcraft-sheet".into())
            .spawn(move || {
                let _ = tx.send(command::run(&cli, &args, command::DEADLINE, &stats));
            });
        if spawned.is_ok() {
            self.job = Some((job, rx));
        } else {
            self.note = Some(Err("the command thread could not be started".to_owned()));
        }
    }

    fn poll(&mut self, cli: &Path, stats: &Arc<Stats>) {
        let Some((job, rx)) = &self.job else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err(ToolError::local(
                "CLI_FAILED",
                "the command thread ended without a result",
            )),
        };
        let job = *job;
        self.job = None;
        let parsed = result.and_then(|output| {
            let reply = decode(&output.stdout)?;
            rejection(&reply, output.code).map_or(Ok(reply), Err)
        });
        match (job, parsed) {
            (Job::Load, Ok(reply)) => match parse_doc(&reply) {
                Some(doc) => {
                    self.pending = movable(&doc);
                    if let Some(furniture) = doc.furniture.clone() {
                        self.sheet_size = furniture.size;
                        self.sheet_project = furniture.project;
                        self.sheet_author = furniture.author;
                        self.sheet_frame = fmt_number(furniture.frame_inset_mm);
                    }
                    self.doc = Some(doc);
                    self.note = None;
                }
                None => {
                    self.note = Some(Err(
                        "the inspect reply is malformed for the sheet editor".to_owned()
                    ));
                }
            },
            (Job::Apply, Ok(_)) => {
                self.note = Some(Ok("Placements applied.".to_owned()));
                // The document moved: read it again so the next edit names the new head.
                if !self.document_dir.is_empty() {
                    self.start(
                        cli,
                        stats,
                        Job::Load,
                        vec![
                            "inspect-drawing".to_owned(),
                            "--document-dir".to_owned(),
                            self.document_dir.clone(),
                        ],
                    );
                }
            }
            (Job::EditSheet { remove }, Ok(_)) => {
                self.note = Some(Ok(if remove {
                    "Sheet removed.".to_owned()
                } else {
                    "Sheet applied.".to_owned()
                }));
                // The document moved: read it again so the next edit names the new head
                // and the form mirrors the sheet the document now carries.
                if !self.document_dir.is_empty() {
                    self.start(
                        cli,
                        stats,
                        Job::Load,
                        vec![
                            "inspect-drawing".to_owned(),
                            "--document-dir".to_owned(),
                            self.document_dir.clone(),
                        ],
                    );
                }
            }
            (_, Err(error)) => {
                self.note = Some(Err(error.to_string()));
            }
        }
    }
}

/// One fresh system identity, when the system source yields one.
pub(crate) fn fresh_id() -> Option<String> {
    IdSource::system().next_id().ok().map(|id| id.to_string())
}

fn fmt_number(value: f64) -> String {
    format!("{value}")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const REPLY: &str = r#"{
        "revision_id": "01900000-0000-7000-8000-000000000001",
        "name": "Member drawing",
        "view": {"id": "p", "plane": "front", "scale_denominator": 100, "paper_origin_mm": [30.0, 40.0]},
        "sheet": {"size": "a3", "frame_inset_mm": 10.0, "project": "Bridge", "author": "Drafter"},
        "placed_views": [
            {"id": "t", "plane": "top", "scale_denominator": 50, "paper_origin_mm": [200.0, 60.0], "title": "PLAN"},
            {"id": "r", "plane": "right", "scale_denominator": 50, "paper_origin_mm": [240.5, 60.4], "title": null}
        ],
        "placed_sections": [
            {"id": "s", "plane": "right", "cut_mm": 1500.0, "side": "positive", "scale_denominator": 5, "paper_origin_mm": [230.0, 250.0], "title": "SECTION A-A"}
        ],
        "placed_details": [
            {"id": "d", "parent": "t", "center_paper_mm": [30.0, 100.0], "radius_mm": 4.0, "scale_denominator": 5, "paper_origin_mm": [230.0, 330.0], "title": null}
        ]
    }"#;

    fn doc() -> Doc {
        let reply =
            beamcraft_json::decode(REPLY.as_bytes(), &beamcraft_json::Limits::DEFAULT).unwrap();
        parse_doc(&reply).unwrap()
    }

    #[test]
    fn the_inspect_reply_reads_into_the_whole_mirror_or_nothing() {
        let doc = doc();
        assert_eq!(doc.revision_id.len(), 36);
        assert_eq!(doc.name, "Member drawing");
        let furniture = doc.furniture.unwrap();
        assert_eq!(furniture.size, "a3");
        assert_eq!(furniture.sheet_mm, [297.0, 420.0]);
        assert_eq!(doc.primary.plane, "front");
        assert_eq!(doc.placed.len(), 2);
        assert_eq!(doc.placed[1].origin, [240.5, 60.4]);
        assert_eq!(doc.placed[0].title.as_deref(), Some("PLAN"));
        assert_eq!(doc.placed[1].title, None);
        assert_eq!(doc.sections.len(), 1);
        assert_eq!(doc.sections[0].cut_mm, 1500.0);
        assert_eq!(doc.sections[0].side, "positive");
        assert_eq!(doc.sections[0].title.as_deref(), Some("SECTION A-A"));
        assert_eq!(doc.details.len(), 1);
        assert_eq!(doc.details[0].parent, "t");
        assert_eq!(doc.details[0].center, [30.0, 100.0]);
        assert_eq!(doc.details[0].radius_mm, 4.0);
        assert_eq!(doc.details[0].title, None);
        // A sheetless document reads with furniture absent, not an error.
        let loose = beamcraft_json::decode(
            br#"{"revision_id":"r","name":"n","view":{"id":"p","plane":"front","scale_denominator":1,"paper_origin_mm":[0,0]},"sheet":null,"placed_views":[],"placed_sections":[],"placed_details":[]}"#,
            &beamcraft_json::Limits::DEFAULT,
        )
        .unwrap();
        let doc = parse_doc(&loose).unwrap();
        assert!(doc.furniture.is_none());
        // A malformed reply is refused, never half-read.
        let broken = beamcraft_json::decode(
            br#"{"revision_id":"r","name":"n","view":{"id":"p"},"sheet":null,"placed_views":[],"placed_sections":[],"placed_details":[]}"#,
            &beamcraft_json::Limits::DEFAULT,
        )
        .unwrap();
        assert!(parse_doc(&broken).is_none());
        // A section reply without its sections key is malformed, never half-read.
        let broken = beamcraft_json::decode(
            br#"{"revision_id":"r","name":"n","view":{"id":"p","plane":"front","scale_denominator":1,"paper_origin_mm":[0,0]},"sheet":null,"placed_views":[],"placed_details":[]}"#,
            &beamcraft_json::Limits::DEFAULT,
        )
        .unwrap();
        assert!(parse_doc(&broken).is_none());
    }

    #[test]
    fn the_placement_args_carry_every_view_kind_and_title() {
        let doc = doc();
        let moved = vec![[205.0, 60.0], [240.5, 60.4], [235.0, 250.0], [230.0, 330.0]];
        let args = placement_args(&doc, &moved);
        assert_eq!(
            args,
            vec![
                "--placed-view",
                "t:top:50:205.00,60.00:PLAN",
                "--placed-view",
                "r:right:50:240.50,60.40",
                "--placed-section",
                "s:right:1500:positive:5:235.00,250.00:SECTION A-A",
                "--placed-detail",
                "d:t:30.00,100.00:4:5:230.00,330.00",
            ]
        );
        // Without pending origins every document origin passes through unchanged.
        let untouched = placement_args(&doc, &[]);
        assert_eq!(
            untouched,
            vec![
                "--placed-view",
                "t:top:50:200.00,60.00:PLAN",
                "--placed-view",
                "r:right:50:240.50,60.40",
                "--placed-section",
                "s:right:1500:positive:5:230.00,250.00:SECTION A-A",
                "--placed-detail",
                "d:t:30.00,100.00:4:5:230.00,330.00",
            ]
        );
    }

    #[test]
    fn moved_origins_stay_in_the_frame_and_on_the_grid() {
        let sheet = [297.0, 420.0];
        assert_eq!(constrain([150.0, 200.0], sheet, 10.0), [150.0, 200.0]);
        // Drag noise rounds to the tenth-millimetre grid.
        assert_eq!(
            constrain([150.3000004, 200.4999999], sheet, 10.0),
            [150.3, 200.5]
        );
        // The frame clamps: a marker never leaves the interior.
        assert_eq!(constrain([-40.0, 1000.0], sheet, 10.0), [10.0, 410.0]);
    }

    #[test]
    fn a_drop_within_tolerance_snaps_onto_the_aligned_axis() {
        let others = [[200.0, 60.0]];
        assert_eq!(snap([200.4, 61.0], &others), [200.0, 61.0]);
        // Each axis snaps on its own: 0.6 mm off is beyond the tolerance and stays.
        assert_eq!(snap([200.4, 60.6], &others), [200.0, 60.6]);
        assert_eq!(snap([201.2, 60.0], &others), [201.2, 60.0]);
        // Both axes align in one gesture.
        assert_eq!(
            snap([240.4, 60.4], &[[240.0, 60.0], [200.0, 60.0]]),
            [240.0, 60.0]
        );
    }

    #[test]
    fn the_sheet_commands_are_the_ones_a_user_types() {
        assert_eq!(
            sheet_args(
                "/d",
                "h",
                "op",
                "rev",
                "act",
                &LaySheet {
                    size: "a4",
                    project: "Bridge",
                    author: "Drafter",
                    frame_inset_mm: None
                }
            ),
            [
                "edit-sheet",
                "--document-dir",
                "/d",
                "--expected-head",
                "h",
                "--op-id",
                "op",
                "--revision-id",
                "rev",
                "--actor-id",
                "act",
                "--sheet-size",
                "a4",
                "--sheet-project",
                "Bridge",
                "--sheet-author",
                "Drafter"
            ]
        );
        // A typed inset names the frame explicitly; empty lets the command default it.
        assert_eq!(
            sheet_args(
                "/d",
                "h",
                "op",
                "rev",
                "act",
                &LaySheet {
                    size: "a3",
                    project: "P",
                    author: "A",
                    frame_inset_mm: Some(12.5)
                }
            )
            .iter()
            .skip_while(|part| part.as_str() != "--sheet-frame")
            .map(String::as_str)
            .collect::<Vec<_>>(),
            ["--sheet-frame", "12.5"]
        );
        assert_eq!(
            remove_sheet_args("/d", "h", "op", "rev", "act"),
            [
                "edit-sheet",
                "--document-dir",
                "/d",
                "--expected-head",
                "h",
                "--op-id",
                "op",
                "--revision-id",
                "rev",
                "--actor-id",
                "act",
                "--sheet-remove",
                "true"
            ]
        );
    }

    #[test]
    fn the_frame_field_reads_empty_as_the_default_and_refuses_junk() {
        assert_eq!(parse_frame(""), Ok(None));
        assert_eq!(parse_frame("   "), Ok(None));
        assert_eq!(parse_frame("10"), Ok(Some(10.0)));
        assert_eq!(parse_frame(" 12.5 "), Ok(Some(12.5)));
        assert!(parse_frame("wide").is_err());
        assert!(parse_frame("nan").is_err());
        assert!(parse_frame("inf").is_err());
    }

    #[test]
    fn the_form_starts_on_the_command_print_default_size() {
        assert_eq!(SheetEditor::default().sheet_size, "a4");
    }
}
