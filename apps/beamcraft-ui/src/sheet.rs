// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D05 sheet editor: hand placement of a drawing document's views on its sheet.
//!
//! The editor reads the document through the same `inspect-drawing` command a user runs,
//! shows the sheet furniture and one marker per placed view, and applies moved origins
//! through the same `edit-views` command with fresh identities from the shared
//! [`IdSource`]. The UI computes no engineering geometry: the reproducible sheet output is
//! the engine's whole-sheet export, reached through the CLI. Dropping a marker within the
//! align tolerance of another view's axis snaps it onto that axis, so aligned views are
//! one gesture, not a numeric edit.

use crate::command::{self, Output};
use crate::stats::Stats;
use crate::tools::{ToolError, decode, rejection, text};
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
}

fn coordinate(json: &Json, key: &str, index: usize) -> Option<f64> {
    json.get(key).ok()?.array().ok()?.get(index)?.float().ok()
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
    })
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
}

/// The sheet editor window state.
#[derive(Default)]
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
        egui::Window::new("Sheet editor")
            .open(&mut open)
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
            ui.label(
                "This document carries no sheet. Lay one on with the CLI's edit-sheet command.",
            );
            self.note_view(ui);
            return;
        };
        ui.separator();
        ui.label(format!(
            "{} — sheet {}, frame {} mm",
            doc.name, furniture.size, furniture.frame_inset_mm
        ));
        self.preview(ui, &doc, furniture);
        ui.horizontal(|ui| {
            let moved = self.pending.iter().enumerate().any(|(index, origin)| {
                doc.placed
                    .get(index)
                    .is_some_and(|placement| placement.origin != *origin)
            });
            if ui
                .add_enabled(
                    moved && self.job.is_none() && !doc.placed.is_empty(),
                    egui::Button::new("Apply placements"),
                )
                .clicked()
            {
                self.note = None;
                self.apply(cli, actor, stats, &doc);
            }
            if ui.add_enabled(moved, egui::Button::new("Reset")).clicked() {
                self.pending = doc.placed.iter().map(|p| p.origin).collect();
            }
        });
        ui.label(format!(
            "{} placed views; the primary view is placed by the document, not here.",
            doc.placed.len()
        ));
        self.note_view(ui);
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
        // Title block: a plain box with the two authored fields, bottom right.
        let block = egui::Rect::from_min_max(
            egui::pos2(frame.right() - 120.0 * scale, frame.bottom() - 44.0 * scale),
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
        if let Some((index, ..)) = &self.drag
            && let Some(origin) = self.pending.get(*index)
        {
            for other in doc
                .placed
                .iter()
                .enumerate()
                .filter(|(other, _)| other != index)
                .map(|(_, placement)| placement.origin)
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
        for (index, placement) in doc.placed.iter().enumerate() {
            let origin = self.pending.get(index).copied().unwrap_or(placement.origin);
            marker(
                painter,
                origin,
                format!(
                    "{} 1:{}",
                    placement.plane,
                    fmt_number(placement.scale_denominator)
                ),
                egui::Color32::from_rgb(20, 90, 200),
            );
        }
        // Drag a placed marker: grab the nearest one under the pointer.
        if let Some(pointer) = response.interact_pointer_pos() {
            let paper = to_paper(pointer);
            if response.drag_started() {
                let hit = doc.placed.iter().enumerate().find(|(index, placement)| {
                    let origin = self
                        .pending
                        .get(*index)
                        .copied()
                        .unwrap_or(placement.origin);
                    let screen = to_screen(origin);
                    ((screen - pointer).length() as f64) <= HIT_PX as f64
                });
                if let Some((index, placement)) = hit {
                    self.drag = Some((index, paper, placement.origin));
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
            let others = doc
                .placed
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != index)
                .map(|(_, placement)| placement.origin)
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

    /// Applies every pending origin through `edit-views` with fresh identities.
    fn apply(&mut self, cli: &Path, actor: Option<&str>, stats: &Arc<Stats>, doc: &Doc) {
        let mut args = vec![
            "edit-views".to_owned(),
            "--document-dir".to_owned(),
            self.document_dir.clone(),
            "--expected-head".to_owned(),
            doc.revision_id.clone(),
        ];
        let Some(op) = fresh_id() else {
            self.note = Some(Err("no identity could be generated".to_owned()));
            return;
        };
        let Some(revision) = fresh_id() else {
            self.note = Some(Err("no identity could be generated".to_owned()));
            return;
        };
        let actor = actor
            .map(str::to_owned)
            .or_else(fresh_id)
            .unwrap_or_default();
        args.extend([
            "--op-id".to_owned(),
            op,
            "--revision-id".to_owned(),
            revision,
            "--actor-id".to_owned(),
            actor,
        ]);
        for placement in &doc.placed {
            let origin = self
                .pending
                .get(index_of(placement, doc))
                .copied()
                .unwrap_or(placement.origin);
            args.extend([
                "--placed-view".to_owned(),
                format!(
                    "{}:{}:{}:{:.2},{:.2}",
                    placement.id,
                    placement.plane,
                    fmt_number(placement.scale_denominator),
                    origin[0],
                    origin[1]
                ),
            ]);
        }
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
                    self.pending = doc
                        .placed
                        .iter()
                        .map(|placement| placement.origin)
                        .collect();
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
            (_, Err(error)) => {
                self.note = Some(Err(error.to_string()));
            }
        }
    }
}

fn index_of(placement: &Placement, doc: &Doc) -> usize {
    doc.placed
        .iter()
        .position(|candidate| candidate.id == placement.id)
        .unwrap_or(usize::MAX)
}

fn fresh_id() -> Option<String> {
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
            {"id": "t", "plane": "top", "scale_denominator": 50, "paper_origin_mm": [200.0, 60.0]},
            {"id": "r", "plane": "right", "scale_denominator": 50, "paper_origin_mm": [240.5, 60.4]}
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
        // A sheetless document reads with furniture absent, not an error.
        let loose = beamcraft_json::decode(
            br#"{"revision_id":"r","name":"n","view":{"id":"p","plane":"front","scale_denominator":1,"paper_origin_mm":[0,0]},"sheet":null,"placed_views":[]}"#,
            &beamcraft_json::Limits::DEFAULT,
        )
        .unwrap();
        let doc = parse_doc(&loose).unwrap();
        assert!(doc.furniture.is_none());
        // A malformed reply is refused, never half-read.
        let broken = beamcraft_json::decode(
            br#"{"revision_id":"r","name":"n","view":{"id":"p"},"sheet":null,"placed_views":[]}"#,
            &beamcraft_json::Limits::DEFAULT,
        )
        .unwrap();
        assert!(parse_doc(&broken).is_none());
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
}
