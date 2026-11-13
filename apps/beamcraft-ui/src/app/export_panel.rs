// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The menu's Export page (V10): the IFC row commits through the same `export-ifc` command a
//! user types. The reply is the CLI's; a refusal writes nothing and is shown on the page, a
//! success reports what the command's own report says.

use super::{App, field};
use crate::command;
use crate::tools::{decode, rejection};
use beamcraft_json::Json;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

/// The longest an output path may be, in bytes; the command itself refuses longer arguments.
const MAX_PATH: usize = 4096;

/// The export form's state between frames; everything else is on [`App`].
pub(crate) struct Form {
    /// Whether the page shows the form.
    open: bool,
    /// Where the IFC file is written, as typed (`--out`).
    pub(super) path: String,
    /// Whether an existing file may be replaced (`--overwrite`).
    overwrite: bool,
    /// The running `export-ifc`, if any.
    pub(super) job: Option<Receiver<Result<String, String>>>,
    /// The last outcome, shown under the form.
    pub(super) note: Option<Result<String, String>>,
}

impl Form {
    pub(super) fn new() -> Self {
        Self {
            open: false,
            path: String::new(),
            overwrite: false,
            job: None,
            note: None,
        }
    }

    /// Shows the form, seeding the output path from the model's own folder when the user has
    /// not typed one.
    pub(crate) fn open(&mut self, store: &Path) {
        self.open = true;
        self.note = None;
        if self.path.trim().is_empty()
            && let Some(suggested) = suggested(store)
        {
            self.path = suggested;
        }
    }

    fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Reads the reply of the running export, if it has arrived.
    fn poll(&mut self) {
        let Some(rx) = &self.job else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(
                "The export command ended without a result; the file may not have been written."
                    .into(),
            ),
        };
        self.job = None;
        self.note = Some(result);
    }

    /// Starts the committed `export-ifc` on a worker: the same command a user types.
    fn start(&mut self, cli: &Path, store: &Path, stats: &Arc<crate::stats::Stats>) {
        let args = match export_args(store, &self.path, self.overwrite) {
            Ok(args) => args,
            Err(message) => {
                self.note = Some(Err(message));
                return;
            }
        };
        let cli = cli.to_path_buf();
        let stats = Arc::clone(stats);
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("beamcraft-export-ifc".into())
            .spawn(move || {
                let result = command::run(&cli, &args, command::DEADLINE, &stats)
                    .map_err(|e| e.to_string())
                    .and_then(|out| {
                        let reply = decode(&out.stdout).map_err(|e| e.to_string())?;
                        rejection(&reply, out.code)
                            .map_or_else(|| Ok(summary(&reply)), |e| Err(e.to_string()))
                    });
                let _ = tx.send(result);
            });
        match spawned {
            Ok(_) => {
                self.job = Some(rx);
                self.note = None;
            }
            Err(_) => self.note = Some(Err("The export command could not be started.".into())),
        }
    }

    /// Draws the form's fields and buttons on the Export page; `outer_busy` disables them while
    /// the window runs a command of its own. `read_only` is a window on a received IFC file:
    /// there is no store to export, so the form only offers the pair's other half, «Open
    /// IFC…» (issue 614). Returns true when «Open IFC…» was clicked.
    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        cli: &Path,
        store: &Path,
        stats: &Arc<crate::stats::Stats>,
        outer_busy: bool,
        read_only: bool,
    ) -> bool {
        let mut open_clicked = false;
        self.poll();
        if self.busy() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }
        let busy = self.busy() || outer_busy;
        let palette = crate::theme::Palette::current(ui);
        ui.add_space(12.0);
        ui.label(crate::theme::bold("IFC", 14.0));
        ui.label(
            egui::RichText::new(if read_only {
                "This window shows a received IFC file. Open another one read-only, or open a \
                 model store to export it."
            } else {
                "Writes the model's accepted revision as an IFC4 STEP file."
            })
            .small()
            .color(palette.muted),
        );
        if !read_only {
            field(ui, "Output file", &mut self.path, 420.0);
            ui.add(egui::Checkbox::new(
                &mut self.overwrite,
                "Replace an existing file",
            ));
            ui.add_space(8.0);
        }
        ui.horizontal(|ui| {
            if crate::theme::button(ui, crate::theme::Kind::Secondary, "Open IFC…", !busy).clicked()
            {
                open_clicked = true;
            }
            if !read_only
                && crate::theme::button(ui, crate::theme::Kind::Primary, "Export IFC", !busy)
                    .clicked()
            {
                self.start(cli, store, stats);
            }
            if crate::theme::button(ui, crate::theme::Kind::Secondary, "Close", true).clicked() {
                self.open = false;
                self.note = None;
            }
            if busy {
                ui.spinner();
                ui.label("Exporting…");
            }
        });
        match &self.note {
            Some(Ok(note)) => {
                ui.add_space(8.0);
                ui.label(egui::RichText::new(note.as_str()).color(palette.muted));
            }
            Some(Err(error)) => {
                ui.add_space(8.0);
                ui.label(egui::RichText::new(error).color(palette.error).strong());
            }
            None => {}
        }
        open_clicked
    }
}

/// The output path the form suggests: the model's own folder and name, with an `.ifc` ending.
fn suggested(store: &Path) -> Option<String> {
    let name = store.file_name()?.to_str()?;
    let parent = store.parent()?;
    Some(
        parent
            .join(format!("{name}.ifc"))
            .to_string_lossy()
            .into_owned(),
    )
}

/// The `export-ifc` arguments of the form, exactly as the command takes them.
pub(crate) fn export_args(store: &Path, out: &str, overwrite: bool) -> Result<Vec<String>, String> {
    let out = out.trim();
    if out.is_empty() {
        return Err("Output file: enter where the IFC file is written".into());
    }
    if out.len() > MAX_PATH {
        return Err(format!("Output file: at most {MAX_PATH} bytes"));
    }
    let mut args = vec![
        "export-ifc".into(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--out".into(),
        out.to_owned(),
    ];
    if overwrite {
        args.push("--overwrite".into());
    }
    Ok(args)
}

/// What the form reports from the command's own report: the status, what was exported and
/// where the bytes went. A reply without the expected fields still says that the file was
/// written, because the command answered success.
fn summary(reply: &Json) -> String {
    let at = |path: &[&str]| path.iter().try_fold(reply, |json, key| json.get(key)).ok();
    let status = at(&["status"]).and_then(|j| j.string().ok());
    let exported = at(&["counts", "exported"]).and_then(|j| j.unsigned().ok());
    let total = at(&["counts", "elements_total"]).and_then(|j| j.unsigned().ok());
    let findings = at(&["total_findings"]).and_then(|j| j.unsigned().ok());
    let bytes = at(&["output", "bytes"]).and_then(|j| j.unsigned().ok());
    let sha = at(&["output", "sha256"])
        .and_then(|j| j.string().ok())
        .map(|sha| sha.chars().take(12).collect::<String>());
    match (exported, total, bytes) {
        (Some(exported), Some(total), Some(bytes)) => {
            let findings = findings.unwrap_or(0);
            let status = status.unwrap_or("complete");
            let sha = sha.map_or_else(String::new, |sha| format!(", sha256 {sha}…"));
            let noun = if findings == 1 { "finding" } else { "findings" };
            format!(
                "Exported {exported} of {total} elements with {findings} {noun} ({status}); \
                 {bytes} bytes{sha}."
            )
        }
        _ => "The model was exported.".to_owned(),
    }
}

impl App {
    /// Opens the Export page's IFC form, as a click on its row does.
    pub(crate) fn open_menu_export(&mut self) {
        self.menu_export.open(&self.source.store);
    }

    /// Whether the Export page's IFC form is shown.
    pub(crate) fn menu_export_open(&self) -> bool {
        self.menu_export.open
    }

    /// The Export page's IFC form; the page calls this while the section is shown.
    pub(crate) fn menu_export_ui(&mut self, ui: &mut egui::Ui, outer_busy: bool) {
        let cli = self.source.cli.clone();
        let store = self.source.store.clone();
        let stats = Arc::clone(&self.stats);
        let read_only = self.source.read_only();
        if self
            .menu_export
            .ui(ui, &cli, &store, &stats, outer_busy, read_only)
            && let Some(start) = self
                .source
                .ifc_source
                .clone()
                .or_else(|| self.source.store.parent().map(Path::to_path_buf))
        {
            // «Open IFC…» (issue 614): the visible entry of the pair next to «Export IFC».
            // The dialog is modal like the start page's folder dialog; a cancelled dialog
            // changes nothing.
            match crate::dialog::pick_file("Open an IFC file", Some(&start)) {
                Ok(Some(file)) => self.open_source_request = Some(file),
                Ok(None) => {}
                Err(e) => self.menu_note = Some(Err(e.to_string())),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_form_builds_the_exact_export_ifc_command() {
        assert_eq!(
            export_args(Path::new("S"), " /m/model.ifc ", false).unwrap(),
            vec!["export-ifc", "--store", "S", "--out", "/m/model.ifc",]
        );
        assert_eq!(
            export_args(Path::new("S"), "/m/model.ifc", true).unwrap()[5..],
            ["--overwrite".to_owned()]
        );
    }

    #[test]
    fn an_empty_or_oversized_path_refuses_without_a_command() {
        assert!(
            export_args(Path::new("S"), "", false)
                .unwrap_err()
                .contains("Output file")
        );
        assert!(
            export_args(Path::new("S"), "   ", false)
                .unwrap_err()
                .contains("Output file")
        );
        assert!(
            export_args(Path::new("S"), &"x".repeat(MAX_PATH + 1), false)
                .unwrap_err()
                .contains("at most")
        );
    }

    #[test]
    fn the_summary_reports_the_command_s_own_facts() {
        let reply = Json::object([
            (
                "counts",
                Json::object([
                    ("exported", Json::uint(2)),
                    ("elements_total", Json::uint(3)),
                ]),
            ),
            ("total_findings", Json::uint(1)),
            ("status", Json::text("incomplete")),
            (
                "output",
                Json::object([
                    ("bytes", Json::uint(4321)),
                    ("sha256", Json::text("0123456789abcdef")),
                ]),
            ),
        ]);
        assert_eq!(
            summary(&reply),
            "Exported 2 of 3 elements with 1 finding (incomplete); 4321 bytes, \
             sha256 0123456789ab…."
        );
        assert_eq!(
            summary(&Json::object([("outcome", Json::text("exported"))])),
            "The model was exported."
        );
    }
}
