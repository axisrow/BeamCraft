// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D06 annotations: hand placement of dimensions, marks and leaders, and their binding
//! states. Placement resolves a feature from the click on the snapshot the window shows and
//! commits through the same `annotation-create` command a user types; the list is the CLI's
//! `annotation-list` with the engine's resolution verdicts.

use super::{App, View};
use crate::command;
use crate::edit::{Ray, feature_pick, ground_point, pick_point_in};
use crate::theme::{BRAND, Palette};
use crate::tools::{ToolError, decode, rejection};
use beamcraft_json::Json;
use egui::Color32;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

/// What a viewport click does while the Annotations bench is active.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Tool {
    /// No placement in progress; the list is the whole panel.
    #[default]
    None,
    /// Two features make a linear dimension.
    Dimension,
    /// One feature and an attribute name make a mark.
    Mark,
    /// One feature, a text and a second click's end point make a leader.
    Leader,
}

impl Tool {
    fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Dimension => "Dimension",
            Self::Mark => "Mark",
            Self::Leader => "Leader",
        }
    }
}

/// One listed annotation with its binding state.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Row {
    /// The element id.
    pub id: String,
    /// The label.
    pub name: String,
    /// `dimension`, `mark` or `leader`.
    pub kind: String,
    /// `current`, `stale`, `unresolved` or `unknown`.
    pub status: String,
    /// Why unresolved.
    pub reason: Option<String>,
    /// The measured distance of a dimension.
    pub value_mm: Option<f64>,
    /// The shown text of a mark or leader.
    pub text: Option<String>,
    /// The resolved world anchors, empty when nothing resolves.
    pub anchors: Vec<[f64; 3]>,
    /// The leader's free end, when the record is a leader.
    pub end_mm: Option<[f64; 3]>,
}

/// The stored state between frames; everything else is on [`App`].
pub(crate) struct Panel {
    pub(crate) tool: Tool,
    /// The first anchor of a two-click placement.
    pub(crate) anchor: Option<String>,
    /// The second click's end point of a leader.
    pub(crate) leader_end: Option<[f64; 3]>,
    pub(crate) attribute: String,
    pub(crate) text: String,
    /// The last listing, or why there is none yet.
    pub(crate) rows: Result<Vec<Row>, String>,
    pub(super) list_job: Option<Receiver<Result<Vec<Row>, ToolError>>>,
    pub(super) commit_job: Option<Receiver<Result<(), ToolError>>>,
    /// Whether the running commit is a delete, so the note names the right action.
    pub(super) deleting: bool,
    pub(crate) note: Option<Result<String, String>>,
}

impl Panel {
    pub(super) fn empty() -> Self {
        Self {
            tool: Tool::None,
            anchor: None,
            leader_end: None,
            attribute: String::new(),
            text: String::new(),
            rows: Ok(Vec::new()),
            list_job: None,
            commit_job: None,
            deleting: false,
            note: None,
        }
    }
}

impl Panel {
    fn busy(&self) -> bool {
        self.list_job.is_some() || self.commit_job.is_some()
    }

    fn clear_placement(&mut self) {
        self.anchor = None;
        self.leader_end = None;
    }
}

/// The `annotation-list` reply as rows; the order is the CLI's (ascending identity).
pub(crate) fn parse_rows(exit_code: Option<i32>, stdout: &[u8]) -> Result<Vec<Row>, ToolError> {
    let reply = decode(stdout)?;
    if let Some(error) = rejection(&reply, exit_code) {
        return Err(error);
    }
    let unreadable = || ToolError {
        code: "UNREADABLE_REPLY".into(),
        message: "the CLI reply lacks a field".into(),
    };
    let entries = reply
        .get("annotations")
        .and_then(|j| j.array())
        .map_err(|_| unreadable())?;
    let text = |a: &Json, key: &str| {
        a.get(key)
            .ok()
            .and_then(|v| v.string().ok())
            .map(str::to_owned)
    };
    let point = |value: &Json| -> Result<[f64; 3], ToolError> {
        let numbers = value.array().map_err(|_| unreadable())?;
        if numbers.len() != 3 {
            return Err(unreadable());
        }
        let mut out = [0.0; 3];
        for (slot, number) in out.iter_mut().zip(numbers) {
            *slot = number.float().map_err(|_| unreadable())?;
        }
        Ok(out)
    };
    entries
        .iter()
        .map(|a| {
            // The CLI writes anchors for every record; an absent key (an older reply) is the
            // same as nothing resolved.
            let anchors = a
                .get("anchors_mm")
                .ok()
                .and_then(|j| j.array().ok())
                .unwrap_or(&[])
                .iter()
                .map(point)
                .collect::<Result<Vec<_>, _>>()?;
            let end_mm = a.get("end_mm").ok().and_then(|v| point(v).ok());
            Ok(Row {
                id: text(a, "id").ok_or_else(unreadable)?,
                name: text(a, "name").ok_or_else(unreadable)?,
                kind: text(a, "kind").ok_or_else(unreadable)?,
                status: text(a, "status").ok_or_else(unreadable)?,
                reason: text(a, "reason"),
                value_mm: a.get("value_mm").ok().and_then(|v| v.float().ok()),
                text: text(a, "bound_text").or_else(|| text(a, "text")),
                anchors,
                end_mm,
            })
        })
        .collect()
}

pub(crate) fn paint(ui: &egui::Ui, rect: egui::Rect, view: &View, origin: [f64; 3], rows: &[Row]) {
    let project = |point: [f64; 3]| {
        let local = std::array::from_fn(|i| {
            (point.get(i).copied().unwrap_or(0.0) - origin.get(i).copied().unwrap_or(0.0)) as f32
        });
        let [x, y, z] = view.project(local);
        ((0.0..1.0).contains(&z) && x.is_finite() && y.is_finite()).then(|| {
            egui::pos2(
                rect.min.x + (x * 0.5 + 0.5) * rect.width(),
                rect.min.y + (0.5 - y * 0.5) * rect.height(),
            )
        })
    };
    let painter = ui.painter_at(rect);
    for row in rows {
        let (stroke, label) = match row.status.as_str() {
            "current" => (
                egui::Stroke::new(1.5, BRAND),
                row.text.clone().or(row.value_mm.map(|v| format!("{v:.1}"))),
            ),
            "stale" => (
                egui::Stroke::new(1.5, Color32::from_rgb(0xE0, 0x8A, 0x00)),
                row.text
                    .clone()
                    .or(row.value_mm.map(|v| format!("{v:.1}?"))),
            ),
            _ => continue,
        };
        match (row.anchors.first().copied(), row.anchors.get(1).copied()) {
            (Some(a), Some(b)) => {
                let (Some(p), Some(q)) = (project(a), project(b)) else {
                    continue;
                };
                painter.line_segment([p, q], stroke);
                if let Some(text) = label
                    && let Some(mid) = project(std::array::from_fn(|i| {
                        (a.get(i).copied().unwrap_or(0.0) + b.get(i).copied().unwrap_or(0.0)) / 2.0
                    }))
                {
                    painter.text(
                        mid + egui::vec2(4.0, -4.0),
                        egui::Align2::LEFT_BOTTOM,
                        format!("{}: {text}", row.name),
                        egui::FontId::proportional(12.0),
                        stroke.color,
                    );
                }
            }
            (Some(a), None) => {
                // A leader runs from its feature to the free end and shows the text there; a
                // mark (or a leader whose end is off screen) is a dot at the feature.
                if let Some(end) = row.end_mm
                    && let (Some(p), Some(q)) = (project(a), project(end))
                {
                    painter.line_segment([p, q], stroke);
                    painter.circle_filled(p, 2.5, stroke.color);
                    if let Some(text) = label {
                        painter.text(
                            q + egui::vec2(6.0, -6.0),
                            egui::Align2::LEFT_BOTTOM,
                            format!("{}: {text}", row.name),
                            egui::FontId::proportional(12.0),
                            stroke.color,
                        );
                    }
                    continue;
                }
                let Some(p) = project(a) else { continue };
                painter.circle_filled(p, 3.0, stroke.color);
                if let Some(text) = label {
                    painter.text(
                        p + egui::vec2(6.0, -6.0),
                        egui::Align2::LEFT_BOTTOM,
                        format!("{}: {text}", row.name),
                        egui::FontId::proportional(12.0),
                        stroke.color,
                    );
                }
            }
            _ => {}
        }
    }
}

impl App {
    /// The inspector panel of the Annotations bench.
    pub(super) fn annotation_panel(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::current(ui);
        ui.heading("Annotations");
        ui.label("Dimensions, marks and leaders bound to features. A lost feature stays unresolved: an address is never re-bound.");
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            for tool in [Tool::Dimension, Tool::Mark, Tool::Leader] {
                if ui
                    .selectable_label(self.annotation.tool == tool, tool.label())
                    .clicked()
                    && !self.other_command_running()
                {
                    self.annotation.tool = if self.annotation.tool == tool {
                        Tool::None
                    } else {
                        tool
                    };
                    self.annotation.clear_placement();
                    self.annotation.note = None;
                }
            }
        });
        if self.annotation.tool != Tool::None {
            match self.annotation.tool {
                Tool::Dimension => {
                    ui.label(match self.annotation.anchor {
                        Some(_) => "Pick the second feature.",
                        None => "Pick the first feature.",
                    });
                }
                Tool::Mark => {
                    crate::app::field(ui, "Attribute", &mut self.annotation.attribute, 140.0);
                    ui.label("Pick the feature to mark; the text is the attribute's value.");
                }
                Tool::Leader => {
                    crate::app::field(ui, "Text", &mut self.annotation.text, 180.0);
                    ui.label(match self.annotation.leader_end {
                        Some(_) => "Pick the feature the leader points at.",
                        None => "Pick the leader's end point, then its feature.",
                    });
                }
                Tool::None => {}
            }
        }
        if let Some(note) = &self.annotation.note {
            match note {
                Ok(message) => ui.label(message.clone()),
                Err(message) => ui.colored_label(palette.error, message.clone()),
            };
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Refresh").clicked() && !self.other_command_running() {
                self.annotation_load();
            }
            if self.annotation.busy() {
                ui.spinner();
            }
        });
        // The rows are copied so a delete can borrow the window mutably inside the loop.
        match self.annotation.rows.clone() {
            Ok(rows) if rows.is_empty() => {
                ui.label("No annotations.");
            }
            Ok(rows) => {
                for row in &rows {
                    ui.horizontal(|ui| {
                        let color = status_color(&palette, &row.status);
                        ui.colored_label(color, row.status.clone());
                        ui.label(format!("{} ({})", row.name, row.kind));
                        match (&row.status, &row.reason, row.value_mm, &row.text) {
                            (s, Some(reason), _, _) if s == "unresolved" => {
                                ui.colored_label(palette.error, reason.clone());
                            }
                            (_, _, Some(value), _) => {
                                ui.label(format!("{value:.1} mm"));
                            }
                            _ => {}
                        }
                        if let Some(text) = &row.text {
                            ui.label(text.clone());
                        }
                        if ui.small_button("Delete").clicked() && !self.other_command_running() {
                            self.annotation_delete(&row.id);
                        }
                    });
                }
            }
            Err(error) => {
                ui.colored_label(palette.error, error.clone());
            }
        }
    }

    /// Resolves a placement click on the viewport.
    pub(crate) fn annotation_click(&mut self, ray: Ray) {
        if self.other_command_running() || self.annotation.busy() {
            return;
        }
        let Some(loaded) = &self.loaded else { return };
        let snapshot = Arc::clone(&loaded.snapshot);
        let tool = self.annotation.tool;
        if tool == Tool::None {
            return;
        }
        if tool == Tool::Leader && self.annotation.leader_end.is_none() {
            // First click: the leader's free end, wherever the click lands.
            let point = pick_point_in(&snapshot, &ray)
                .ok()
                .flatten()
                .map(|(point, _)| point)
                .or_else(|| ground_point(&ray));
            match point {
                Some(point) => self.annotation.leader_end = Some(point),
                None => {
                    self.annotation.note = Some(Err("Aim at the model or the ground plane.".into()))
                }
            }
            return;
        }
        let picked = feature_pick(&snapshot, &ray);
        let loaded = match picked {
            Ok(Some((feature, _))) => feature,
            Ok(None) => {
                self.annotation.note =
                    Some(Err("No named feature was hit; pick a model face.".into()));
                return;
            }
            Err(error) => {
                self.annotation.note = Some(Err(error.to_string()));
                return;
            }
        };
        let store = self.source.store.clone();
        let cli = self.source.cli.clone();
        let stats = Arc::clone(&self.stats);
        let names: Vec<String> = self
            .annotation
            .rows
            .as_ref()
            .map(|rows| rows.iter().map(|r| r.name.clone()).collect())
            .unwrap_or_default();
        match tool {
            Tool::Dimension if self.annotation.anchor.is_none() => {
                self.annotation.anchor = Some(loaded);
            }
            Tool::Dimension | Tool::Mark | Tool::Leader => {
                let from = if tool == Tool::Dimension {
                    self.annotation.anchor.take().unwrap_or_default()
                } else {
                    loaded.clone()
                };
                let to = (tool == Tool::Dimension).then_some(loaded);
                let (attribute, text, end) = match tool {
                    Tool::Mark => (self.annotation.attribute.clone(), None, None),
                    Tool::Leader => (
                        String::new(),
                        Some(self.annotation.text.clone()),
                        self.annotation.leader_end.take(),
                    ),
                    _ => (String::new(), None, None),
                };
                let place = Placement {
                    kind: match tool {
                        Tool::Dimension => "dimension",
                        Tool::Mark => "mark",
                        _ => "leader",
                    },
                    from: &from,
                    to: to.as_deref(),
                    attribute: &attribute,
                    text: text.as_deref(),
                    end,
                };
                let name = fresh_name(&names, place.kind);
                let args = create_args(&store, &name, &place);
                let (tx, rx) = channel();
                let started = std::thread::Builder::new()
                    .name("beamcraft-annotation-create".into())
                    .spawn(move || {
                        let result =
                            command::run(&cli, &args, command::DEADLINE, &stats).and_then(|out| {
                                let reply = decode(&out.stdout)?;
                                rejection(&reply, out.code).map_or(Ok(()), Err)
                            });
                        let _ = tx.send(result);
                    });
                match started {
                    Ok(_) => {
                        self.annotation.commit_job = Some(rx);
                        self.annotation.note = None;
                    }
                    Err(_) => {
                        self.annotation.note =
                            Some(Err("The annotation command could not be started.".into()))
                    }
                }
            }
            Tool::None => {}
        }
    }

    /// Starts an `annotation-list` worker and keeps the last rows until it answers.
    pub(crate) fn annotation_load(&mut self) {
        if self.other_command_running() || self.annotation.list_job.is_some() {
            return;
        }
        let cli = self.source.cli.clone();
        let stats = Arc::clone(&self.stats);
        let args = vec![
            "annotation-list".to_owned(),
            "--store".to_owned(),
            self.source.store.to_string_lossy().into_owned(),
        ];
        let (tx, rx) = channel();
        match std::thread::Builder::new()
            .name("beamcraft-annotation-list".into())
            .spawn(move || {
                let result = command::run(&cli, &args, Duration::from_secs(30), &stats)
                    .and_then(|out| parse_rows(out.code, &out.stdout));
                let _ = tx.send(result);
            }) {
            Ok(_) => self.annotation.list_job = Some(rx),
            Err(_) => self.annotation.rows = Err("The list command could not be started.".into()),
        }
    }

    fn annotation_delete(&mut self, id: &str) {
        self.annotation.deleting = true;
        let cli = self.source.cli.clone();
        let stats = Arc::clone(&self.stats);
        let args = vec![
            "delete-element".to_owned(),
            "--store".to_owned(),
            self.source.store.to_string_lossy().into_owned(),
            "--id".to_owned(),
            id.to_owned(),
        ];
        let (tx, rx) = channel();
        match std::thread::Builder::new()
            .name("beamcraft-annotation-delete".into())
            .spawn(move || {
                let result = command::run(&cli, &args, command::DEADLINE, &stats).and_then(|out| {
                    let reply = decode(&out.stdout)?;
                    rejection(&reply, out.code).map_or(Ok(()), Err)
                });
                let _ = tx.send(result);
            }) {
            Ok(_) => {
                self.annotation.commit_job = Some(rx);
                self.annotation.note = None;
            }
            Err(_) => self.annotation.note = Some(Err("The delete could not be started.".into())),
        }
    }

    /// Drains the annotation jobs; a successful commit refreshes the list.
    pub(crate) fn poll_annotations(&mut self) {
        if let Some(rx) = &self.annotation.list_job {
            let result = match rx.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => Err(ToolError::local(
                    "CLI_FAILED",
                    "The list command ended without a result.",
                )),
            };
            self.annotation.list_job = None;
            self.annotation.rows = result.map_err(|e| e.to_string());
        }
        let Some(rx) = &self.annotation.commit_job else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(ToolError::local(
                "CLI_FAILED",
                "The command ended without a result.",
            )),
        };
        self.annotation.commit_job = None;
        let was_delete = self.annotation.deleting;
        self.annotation.deleting = false;
        match result {
            Ok(()) => {
                self.annotation.clear_placement();
                self.annotation.note = Some(Ok(if was_delete {
                    "Deleted.".into()
                } else {
                    "Placed.".into()
                }));
                self.annotation_load();
            }
            Err(error) => {
                self.annotation.note = Some(Err(error.to_string()));
                self.annotation_load();
            }
        }
    }
}

/// A label that is one past every existing label with the same prefix.
fn fresh_name(names: &[String], prefix: &str) -> String {
    let top = names
        .iter()
        .filter(|n| n.starts_with(prefix))
        .filter_map(|n| n[prefix.len()..].parse::<usize>().ok())
        .max()
        .unwrap_or(0);
    format!("{prefix}{top}", top = top + 1)
}

/// Everything one placement needs, so `create_args` stays a small formatter.
struct Placement<'a> {
    kind: &'static str,
    from: &'a str,
    to: Option<&'a str>,
    attribute: &'a str,
    text: Option<&'a str>,
    end: Option<[f64; 3]>,
}

/// The `annotation-create` arguments for one placement.
fn create_args(store: &Path, name: &str, place: &Placement<'_>) -> Vec<String> {
    let mut args = vec![
        "annotation-create".to_owned(),
        "--store".to_owned(),
        store.to_string_lossy().into_owned(),
        "--name".to_owned(),
        name.to_owned(),
        "--kind".to_owned(),
        place.kind.to_owned(),
    ];
    match place.kind {
        "dimension" => {
            args.extend(["--from".into(), place.from.to_owned()]);
            if let Some(to) = place.to {
                args.extend(["--to".into(), to.to_owned()]);
            }
        }
        "mark" => {
            args.extend(["--anchor".into(), place.from.to_owned()]);
            args.extend(["--attribute".into(), place.attribute.to_owned()]);
        }
        _ => {
            args.extend(["--anchor".into(), place.from.to_owned()]);
            if let Some(text) = place.text {
                args.extend(["--text".into(), text.to_owned()]);
            }
            if let Some([x, y, z]) = place.end {
                args.extend(["--end".into(), format!("{x},{y},{z}")]);
            }
        }
    }
    args
}

fn status_color(palette: &Palette, status: &str) -> Color32 {
    match status {
        "current" => BRAND,
        "stale" => Color32::from_rgb(0xE0, 0x8A, 0x00),
        "unresolved" => palette.error,
        _ => palette.muted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listing_becomes_rows_with_their_binding_states() {
        // The exact CLI shape: an unresolved record carries no anchors, a leader its free end.
        let reply = br#"{"annotations":[
            {"id":"a","name":"DIM1","kind":"dimension","status":"current",
             "value_mm":1200,"anchors_mm":[[0,300,5],[1200,300,5]]},
            {"id":"b","name":"LD1","kind":"leader","status":"stale",
             "text":"note","end_mm":[1500,900,50],"anchors_mm":[[0,300,10]]},
            {"id":"c","name":"MK1","kind":"mark","status":"unresolved",
             "reason":"feature-missing","anchors_mm":[]}
        ]}"#;
        let rows = parse_rows(Some(0), reply).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].value_mm, Some(1200.0));
        assert_eq!(
            rows[0].anchors,
            vec![[0.0, 300.0, 5.0], [1200.0, 300.0, 5.0]]
        );
        assert_eq!(rows[1].end_mm, Some([1500.0, 900.0, 50.0]));
        assert_eq!(rows[1].text.as_deref(), Some("note"));
        assert_eq!(rows[2].reason.as_deref(), Some("feature-missing"));
        assert!(rows[2].anchors.is_empty());
        assert!(parse_rows(Some(1), br#"{"outcome":"rejected"}"#).is_err());
    }

    #[test]
    fn a_reply_without_anchors_is_an_empty_row_not_a_refusal() {
        let reply = br#"{"annotations":[
            {"id":"c","name":"MK1","kind":"mark","status":"unresolved",
             "reason":"element-missing"}
        ]}"#;
        let rows = parse_rows(Some(0), reply).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].anchors.is_empty());
    }

    #[test]
    fn a_fresh_label_is_one_past_its_prefix() {
        assert_eq!(fresh_name(&[], "DIM"), "DIM1");
        assert_eq!(fresh_name(&["DIM1".into(), "MK9".into()], "DIM"), "DIM2");
        assert_eq!(fresh_name(&["DIM7".into()], "LD"), "LD1");
    }
}
