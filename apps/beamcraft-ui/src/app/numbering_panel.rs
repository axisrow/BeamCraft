// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The Numbering bench (#846): preview part and assembly numbering (`preview-numbering`),
//! inspect stored approval status (`inspect-numbering`), and commit approved marks to the
//! model (`approve-numbering`) after reviewing the interactive preview table.

use super::App;
use crate::command;
use crate::tools::{ToolError, decode, rejection, text};
use beamcraft_json::Json;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

/// One position group returned by `preview-numbering`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GroupRow {
    /// Position mark (`P1`, `B1`, `C1`, `A-1`, `Б1`, ...).
    pub(crate) position: String,
    /// Kind (`plate`, `beam`, `column`, `assembly`).
    pub(crate) kind: String,
    /// Number of equivalent elements in the group.
    pub(crate) count: u64,
    /// Group status (`new` or `overlapping`).
    pub(crate) status: String,
    /// Stored mark comparison (`same`, `different`, `none`, `mixed`).
    pub(crate) stored: String,
    /// Reference element ID.
    pub(crate) reference: String,
    /// Member element IDs (up to 32 returned per page).
    pub(crate) members: Vec<String>,
    /// Human-readable description of the group.
    pub(crate) description: String,
}

/// Parsed result of `preview-numbering`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreviewReport {
    /// SHA-256 preview token required by `approve-numbering --preview-token`.
    pub(crate) preview_token: String,
    /// Approval state (`current`, `stale`, `not_approved`, ...).
    pub(crate) state: String,
    /// Elements with stored marks.
    pub(crate) marked: u64,
    /// Elements without stored marks.
    pub(crate) unmarked: u64,
    /// Elements whose stored mark differs from the proposal.
    pub(crate) differing: u64,
    /// Proposed position groups.
    pub(crate) groups: Vec<GroupRow>,
}

/// State of the Numbering bench between frames.
pub(crate) struct Panel {
    /// Plate numbering series prefix and start number.
    pub(crate) plate_prefix: String,
    pub(crate) plate_start: String,
    /// Beam numbering series prefix and start number.
    pub(crate) beam_prefix: String,
    pub(crate) beam_start: String,
    /// Column numbering series prefix and start number.
    pub(crate) column_prefix: String,
    pub(crate) column_start: String,
    /// Assembly numbering series prefix and start number.
    pub(crate) assembly_prefix: String,
    pub(crate) assembly_start: String,
    /// `--tolerance` in mm.
    pub(crate) tolerance: String,
    /// `--compare-name`.
    pub(crate) compare_name: bool,
    /// `--compare-holes`.
    pub(crate) compare_holes: bool,
    /// Whether the interactive preview table window is open.
    pub(crate) preview_open: bool,
    /// Row currently highlighted in the preview table.
    pub(crate) selected_group: Option<usize>,
    /// Latest preview report.
    pub(crate) report: Option<PreviewReport>,
    pub(super) preview_job: Option<Receiver<Result<PreviewReport, ToolError>>>,
    pub(super) approve_job: Option<Receiver<Result<String, ToolError>>>,
    /// Last outcome or refusal message.
    pub(crate) note: Option<Result<String, String>>,
}

impl Panel {
    pub(super) fn empty() -> Self {
        Self {
            plate_prefix: "PL".into(),
            plate_start: "1".into(),
            beam_prefix: "B".into(),
            beam_start: "1".into(),
            column_prefix: "C".into(),
            column_start: "1".into(),
            assembly_prefix: "A".into(),
            assembly_start: "1".into(),
            tolerance: "0.1".into(),
            compare_name: false,
            compare_holes: true,
            preview_open: false,
            selected_group: None,
            report: None,
            preview_job: None,
            approve_job: None,
            note: None,
        }
    }

    pub(crate) fn busy(&self) -> bool {
        self.preview_job.is_some() || self.approve_job.is_some()
    }

    pub(crate) fn plate_series(&self) -> String {
        format!("{}:{}", self.plate_prefix.trim(), self.plate_start.trim())
    }

    pub(crate) fn beam_series(&self) -> String {
        format!("{}:{}", self.beam_prefix.trim(), self.beam_start.trim())
    }

    pub(crate) fn column_series(&self) -> String {
        format!("{}:{}", self.column_prefix.trim(), self.column_start.trim())
    }

    pub(crate) fn assembly_series(&self) -> String {
        format!(
            "{}:{}",
            self.assembly_prefix.trim(),
            self.assembly_start.trim()
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn push_series_options(
    args: &mut Vec<String>,
    plate: &str,
    beam: &str,
    column: &str,
    assembly: &str,
    tolerance: &str,
    compare_name: bool,
    compare_holes: bool,
) -> Result<(), String> {
    for (label, val, flag) in [
        ("Plate series", plate, "--plate-series"),
        ("Beam series", beam, "--beam-series"),
        ("Column series", column, "--column-series"),
        ("Assembly series", assembly, "--assembly-series"),
    ] {
        let val = val.trim();
        if !val.is_empty() {
            if !val.contains(':') {
                return Err(format!("{label}: enter PREFIX:START (for example PL:1)."));
            }
            args.extend([flag.to_owned(), val.to_owned()]);
        }
    }
    let tol = tolerance.trim();
    if !tol.is_empty() {
        let parsed = tol
            .parse::<f64>()
            .map_err(|_| format!("Tolerance: \"{tol}\" is not a valid number."))?;
        if !parsed.is_finite() || parsed < 0.0 {
            return Err("Tolerance: enter a non-negative number of millimetres.".into());
        }
        args.extend(["--tolerance".into(), tol.to_owned()]);
    }
    args.extend([
        "--compare-name".into(),
        if compare_name { "true" } else { "false" }.into(),
        "--compare-holes".into(),
        if compare_holes { "true" } else { "false" }.into(),
    ]);
    Ok(())
}

/// Builds the CLI arguments for `preview-numbering`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn preview_args(
    store: &Path,
    plate: &str,
    beam: &str,
    column: &str,
    assembly: &str,
    tolerance: &str,
    compare_name: bool,
    compare_holes: bool,
) -> Result<Vec<String>, String> {
    let mut args = vec![
        "preview-numbering".into(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
    ];
    push_series_options(
        &mut args,
        plate,
        beam,
        column,
        assembly,
        tolerance,
        compare_name,
        compare_holes,
    )?;
    Ok(args)
}

/// Builds the CLI arguments for `approve-numbering`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn approve_args(
    store: &Path,
    actor: Option<&str>,
    preview_token: &str,
    plate: &str,
    beam: &str,
    column: &str,
    assembly: &str,
    tolerance: &str,
    compare_name: bool,
    compare_holes: bool,
) -> Result<Vec<String>, String> {
    let token = preview_token.trim();
    if token.is_empty() {
        return Err("Generate preview first to obtain a preview token.".into());
    }
    let mut args = vec![
        "approve-numbering".into(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--preview-token".into(),
        token.to_owned(),
    ];
    push_series_options(
        &mut args,
        plate,
        beam,
        column,
        assembly,
        tolerance,
        compare_name,
        compare_holes,
    )?;
    if let Some(actor) = actor {
        args.extend(["--actor-id".into(), actor.to_owned()]);
    }
    Ok(args)
}

/// Parses the JSON output of `preview-numbering`.
pub(crate) fn parse_preview(
    exit_code: Option<i32>,
    stdout: &[u8],
) -> Result<PreviewReport, ToolError> {
    let reply = decode(stdout)?;
    if let Some(error) = rejection(&reply, exit_code) {
        return Err(error);
    }
    let preview_token = text(&reply, "preview_token")
        .ok_or_else(|| ToolError::local("UNREADABLE_REPLY", "missing preview_token"))?;
    let approval = reply
        .get("approval")
        .map_err(|_| ToolError::local("UNREADABLE_REPLY", "missing approval"))?;
    let state = text(approval, "state").unwrap_or_else(|| "unknown".into());
    let marked = approval.get("marked").and_then(Json::unsigned).unwrap_or(0);
    let unmarked = approval
        .get("unmarked")
        .and_then(Json::unsigned)
        .unwrap_or(0);
    let differing = approval
        .get("differing")
        .and_then(Json::unsigned)
        .unwrap_or(0);
    let raw_groups = reply
        .get("groups")
        .and_then(Json::array)
        .map_err(|_| ToolError::local("UNREADABLE_REPLY", "missing groups array"))?;
    let mut groups = Vec::with_capacity(raw_groups.len());
    for g in raw_groups {
        let members = g
            .get("members")
            .and_then(Json::array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.string().ok().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        groups.push(GroupRow {
            position: text(g, "position").unwrap_or_default(),
            kind: text(g, "kind").unwrap_or_default(),
            count: g.get("count").and_then(Json::unsigned).unwrap_or(0),
            status: text(g, "status").unwrap_or_default(),
            stored: text(g, "stored").unwrap_or_default(),
            reference: text(g, "reference").unwrap_or_default(),
            members,
            description: text(g, "description").unwrap_or_default(),
        });
    }
    Ok(PreviewReport {
        preview_token,
        state,
        marked,
        unmarked,
        differing,
        groups,
    })
}

impl App {
    /// Starts a `preview-numbering` run on a background worker and opens the preview table.
    pub(crate) fn numbering_preview(&mut self) {
        if self.source.read_only() || self.numbering.busy() {
            return;
        }
        let args = match preview_args(
            &self.source.store,
            &self.numbering.plate_series(),
            &self.numbering.beam_series(),
            &self.numbering.column_series(),
            &self.numbering.assembly_series(),
            &self.numbering.tolerance,
            self.numbering.compare_name,
            self.numbering.compare_holes,
        ) {
            Ok(args) => args,
            Err(msg) => {
                self.numbering.note = Some(Err(msg));
                return;
            }
        };
        let cli = self.source.cli.clone();
        let stats = Arc::clone(&self.stats);
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("beamcraft-numbering-preview".into())
            .spawn(move || {
                let res = command::run(&cli, &args, command::DEADLINE, &stats)
                    .and_then(|out| parse_preview(out.code, &out.stdout));
                let _ = tx.send(res);
            });
        match spawned {
            Ok(_) => {
                self.numbering.preview_job = Some(rx);
                self.numbering.note = None;
            }
            Err(_) => {
                self.numbering.note =
                    Some(Err("The numbering preview could not be started.".into()));
            }
        }
    }

    /// Starts an `approve-numbering` run on a background worker.
    pub(crate) fn numbering_approve(&mut self) {
        if self.other_command_running() || self.numbering.busy() {
            self.numbering.note = Some(Err("Another command is running; wait for it.".into()));
            return;
        }
        let token = self
            .numbering
            .report
            .as_ref()
            .map_or("", |r| r.preview_token.as_str());
        let args = match approve_args(
            &self.source.store,
            self.source.actor.as_deref(),
            token,
            &self.numbering.plate_series(),
            &self.numbering.beam_series(),
            &self.numbering.column_series(),
            &self.numbering.assembly_series(),
            &self.numbering.tolerance,
            self.numbering.compare_name,
            self.numbering.compare_holes,
        ) {
            Ok(args) => args,
            Err(msg) => {
                self.numbering.note = Some(Err(msg));
                return;
            }
        };
        let cli = self.source.cli.clone();
        let stats = Arc::clone(&self.stats);
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("beamcraft-numbering-approve".into())
            .spawn(move || {
                let res = command::run(&cli, &args, command::DEADLINE, &stats).and_then(|out| {
                    let reply = decode(&out.stdout)?;
                    if let Some(err) = rejection(&reply, out.code) {
                        return Err(err);
                    }
                    let marks = reply.get("marks").and_then(Json::unsigned).unwrap_or(0);
                    let groups = reply.get("groups").and_then(Json::unsigned).unwrap_or(0);
                    Ok(format!(
                        "Numbering applied: {marks} mark(s) across {groups} group(s)."
                    ))
                });
                let _ = tx.send(res);
            });
        match spawned {
            Ok(_) => {
                self.numbering.approve_job = Some(rx);
                self.numbering.note = None;
            }
            Err(_) => {
                self.numbering.note =
                    Some(Err("The numbering approval could not be started.".into()));
            }
        }
    }

    /// Highlights all parts belonging to preview group `index` in the 3D viewport.
    fn select_numbering_group(&mut self, index: usize) {
        let Some(group) = self
            .numbering
            .report
            .as_ref()
            .and_then(|r| r.groups.get(index))
            .cloned()
        else {
            return;
        };
        self.numbering.selected_group = Some(index);
        self.drop_grid_edit();

        let mut highlight_ids: Vec<String> = Vec::new();
        if group.kind == "assembly" {
            for asm_id in &group.members {
                if let Some(row) = self.assembly.list.iter().find(|a| &a.id == asm_id) {
                    if let Some(main) = &row.main
                        && !highlight_ids.contains(main)
                    {
                        highlight_ids.push(main.clone());
                    }
                    for part in &row.parts {
                        if !highlight_ids.contains(part) {
                            highlight_ids.push(part.clone());
                        }
                    }
                }
            }
        } else {
            highlight_ids = group.members.clone();
        }
        self.selected_many = highlight_ids.clone();
        if let Some(primary) = highlight_ids.first().cloned() {
            self.editor.select_id(primary);
            self.refresh_selection();
        }
    }

    /// Polls background preview and approval jobs for the Numbering bench.
    pub(crate) fn poll_numbering(&mut self) {
        if let Some(rx) = &self.numbering.preview_job {
            match rx.try_recv() {
                Ok(Ok(report)) => {
                    self.numbering.preview_job = None;
                    let summary = format!(
                        "Preview ready: {} group(s) · status: {}.",
                        report.groups.len(),
                        report.state
                    );
                    self.numbering.selected_group = None;
                    self.numbering.report = Some(report);
                    self.numbering.preview_open = true;
                    self.numbering.note = Some(Ok(summary));
                }
                Ok(Err(err)) => {
                    self.numbering.preview_job = None;
                    self.numbering.note = Some(Err(err.to_string()));
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.numbering.preview_job = None;
                }
            }
        }
        let Some(rx) = &self.numbering.approve_job else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(res) => res,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(ToolError::local(
                "CLI_FAILED",
                "The numbering approval ended without a result.",
            )),
        };
        self.numbering.approve_job = None;
        match result {
            Ok(msg) => {
                self.numbering.note = Some(Ok(msg));
                self.numbering_preview();
                self.reload();
            }
            Err(error) => {
                if error.code == "OUTCOME_UNKNOWN" {
                    self.numbering_preview();
                    self.reload();
                }
                self.numbering.note = Some(Err(error.to_string()));
            }
        }
    }

    /// Bottom action bar for `Bench::Numbering`.
    pub(crate) fn numbering_actions(&mut self, ui: &mut egui::Ui) {
        let busy = self.numbering.busy() || self.other_command_running();
        let done = crate::kit::actions(ui, "Generate preview", !busy, "Cancel", busy);
        if done.primary {
            self.numbering_preview();
        }
        if done.cancel {
            self.set_bench(super::workbench::Bench::Select);
        }
    }

    /// Renders the Numbering inspector panel.
    pub(crate) fn numbering_panel(&mut self, ui: &mut egui::Ui) {
        use crate::kit;
        use crate::theme::{self, Icon, Kind, Palette};
        let palette = Palette::current(ui);
        ui.set_max_width(ui.available_width());
        kit::type_header(ui, Icon::Numbering, "Numbering", |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::icon_button(
                    ui,
                    Icon::Refresh,
                    "Generate preview",
                    !self.numbering.busy() && !self.other_command_running(),
                )
                .clicked()
                {
                    self.numbering_preview();
                }
            });
        });

        kit::section(ui, "num", "Comparison rules", true, |ui| {
            kit::prop_row(ui, "Compare holes", |ui| {
                ui.checkbox(&mut self.numbering.compare_holes, "Separate by holes");
            });
            kit::prop_row(ui, "Compare name", |ui| {
                ui.checkbox(&mut self.numbering.compare_name, "Separate by part name");
            });
            super::field(ui, "Tolerance, mm", &mut self.numbering.tolerance, 90.0);
        });

        kit::section(ui, "num", "Numbering series", true, |ui| {
            let mut series_row = |label: &str, prefix: &mut String, start: &mut String| {
                kit::prop_row(ui, label, |ui| {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(prefix)
                                .desired_width(64.0)
                                .hint_text("Prefix"),
                        );
                        ui.label(egui::RichText::new(":").color(palette.muted));
                        ui.add(
                            egui::TextEdit::singleline(start)
                                .desired_width(48.0)
                                .hint_text("1"),
                        );
                    });
                });
            };
            series_row(
                "Plate",
                &mut self.numbering.plate_prefix,
                &mut self.numbering.plate_start,
            );
            series_row(
                "Beam",
                &mut self.numbering.beam_prefix,
                &mut self.numbering.beam_start,
            );
            series_row(
                "Column",
                &mut self.numbering.column_prefix,
                &mut self.numbering.column_start,
            );
            series_row(
                "Assembly",
                &mut self.numbering.assembly_prefix,
                &mut self.numbering.assembly_start,
            );
        });

        if let Some(report) = &self.numbering.report {
            kit::section(ui, "num", "Last preview", true, |ui| {
                kit::prop_info(ui, "Status", Some(&report.state), "—");
                let counts = format!(
                    "{} group(s) · {} marked · {} unmarked · {} differing",
                    report.groups.len(),
                    report.marked,
                    report.unmarked,
                    report.differing
                );
                kit::prop_info(ui, "Summary", Some(&counts), "—");
                kit::prop_row(ui, "", |ui| {
                    if theme::button(
                        ui,
                        Kind::Secondary,
                        "Open preview table",
                        !self.numbering.preview_open,
                    )
                    .clicked()
                    {
                        self.numbering.preview_open = true;
                    }
                });
            });
        }
    }

    /// Floating interactive preview table window: click any row to highlight it in 3D,
    /// then press Apply numbering at the bottom to store the marks.
    pub(crate) fn numbering_preview_window(&mut self, ctx: &egui::Context) {
        if !self.numbering.preview_open {
            return;
        }
        let mut open = self.numbering.preview_open;
        let mut clicked_row: Option<usize> = None;
        let mut apply = false;
        let mut close = false;
        let busy = self.numbering.busy() || self.other_command_running();

        crate::kit::tool_window("Numbering preview", &mut open, ctx)
            .resizable(true)
            .default_width(640.0)
            .default_height(420.0)
            .show(ctx, |ui| {
                let palette = crate::theme::Palette::current(ui);
                let Some(report) = self.numbering.report.clone() else {
                    ui.label("Run Generate preview to inspect proposed marks.");
                    return;
                };

                egui::Panel::bottom("numbering-preview-actions")
                    .frame(
                        egui::Frame::new()
                            .fill(palette.surface)
                            .inner_margin(egui::Margin::symmetric(0, 4)),
                    )
                    .show(ui, |ui| {
                        let has_groups = !report.groups.is_empty();
                        let done = crate::kit::actions(
                            ui,
                            "Apply numbering",
                            has_groups && !busy,
                            "Close",
                            busy,
                        );
                        apply = done.primary;
                        close = done.cancel;
                    });

                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "Status: {}   ·   {} group(s)   ·   {} marked   ·   {} unmarked   ·   {} differing",
                            report.state,
                            report.groups.len(),
                            report.marked,
                            report.unmarked,
                            report.differing
                        ))
                        .size(12.0)
                        .color(palette.text),
                    );
                });
                ui.label(
                    egui::RichText::new(
                        "Click any row below to highlight its parts or assemblies in the 3D view before applying.",
                    )
                    .size(11.0)
                    .color(palette.muted),
                );
                ui.add_space(4.0);
                ui.separator();

                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Grid::new("numbering-preview-table")
                            .num_columns(6)
                            .striped(true)
                            .spacing([12.0, 6.0])
                            .show(ui, |ui| {
                                for h in ["Position", "Kind", "Qty", "Proposal", "Stored", "Details"]
                                {
                                    ui.label(
                                        egui::RichText::new(h)
                                            .size(11.0)
                                            .strong()
                                            .color(palette.muted),
                                    );
                                }
                                ui.end_row();

                                for (idx, g) in report.groups.iter().enumerate() {
                                    let selected = self.numbering.selected_group == Some(idx);
                                    let pos_resp = ui.selectable_label(
                                        selected,
                                        egui::RichText::new(&g.position).strong(),
                                    );
                                    let kind_resp = ui.selectable_label(selected, &g.kind);
                                    let qty_resp =
                                        ui.selectable_label(selected, g.count.to_string());
                                    let status_color = if g.status == "overlapping" {
                                        palette.error
                                    } else {
                                        palette.text
                                    };
                                    let status_resp = ui.selectable_label(
                                        selected,
                                        egui::RichText::new(&g.status).color(status_color),
                                    );
                                    let stored_resp = ui.selectable_label(selected, &g.stored);
                                    let desc_resp = ui.selectable_label(selected, &g.description);
                                    ui.end_row();
                                    if pos_resp.clicked()
                                        || kind_resp.clicked()
                                        || qty_resp.clicked()
                                        || status_resp.clicked()
                                        || stored_resp.clicked()
                                        || desc_resp.clicked()
                                    {
                                        clicked_row = Some(idx);
                                    }
                                }
                            });
                    });
            });

        self.numbering.preview_open = open && !close;
        if let Some(idx) = clicked_row {
            self.select_numbering_group(idx);
        }
        if apply {
            self.numbering_approve();
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn numbering_command_builders_match_cli_contract() {
        let store = Path::new("model");
        let prev = preview_args(store, "П:1", "Б:1", "К:1", "Ф:1", "1.0", true, false).unwrap();
        assert_eq!(
            prev,
            [
                "preview-numbering",
                "--store",
                "model",
                "--plate-series",
                "П:1",
                "--beam-series",
                "Б:1",
                "--column-series",
                "К:1",
                "--assembly-series",
                "Ф:1",
                "--tolerance",
                "1.0",
                "--compare-name",
                "true",
                "--compare-holes",
                "false",
            ]
        );
        let app = approve_args(
            store,
            Some("actor-1"),
            "tok123",
            "П:1",
            "Б:1",
            "К:1",
            "Ф:1",
            "1",
            false,
            true,
        )
        .unwrap();
        assert_eq!(app[0], "approve-numbering");
        assert!(app.contains(&"--preview-token".to_owned()));
        assert!(app.contains(&"tok123".to_owned()));
        assert!(app.contains(&"--compare-holes".to_owned()));
    }

    #[test]
    fn parse_preview_reads_token_and_groups() {
        let json = br#"{"outcome":"numbering_preview","revision_id":"r1","preview_token":"abc123","approval":{"state":"unapproved","marked":0,"unmarked":2,"differing":0},"groups":[{"kind":"beam","position":"B1","number":1,"status":"ok","count":2,"reference":"id-1","members":["id-1","id-2"],"description":"IPE200","stored":"unmarked"}]}"#;
        let parsed = parse_preview(Some(0), json).unwrap();
        assert_eq!(parsed.preview_token, "abc123");
        assert_eq!(parsed.state, "unapproved");
        assert_eq!(parsed.unmarked, 2);
        assert_eq!(parsed.groups.len(), 1);
        assert_eq!(parsed.groups[0].position, "B1");
        assert_eq!(parsed.groups[0].count, 2);
        assert_eq!(parsed.groups[0].members, vec!["id-1", "id-2"]);
    }
}
