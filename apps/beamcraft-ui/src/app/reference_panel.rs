// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The USD Reference Models bench (#850): attach a read-only external USD file
//! (`attach-reference`), inspect attached reference wrappers (`inspect-reference`), move or
//! orient a wrapper (`move-reference`), adjust wrapper unit interpretation
//! (`correct-reference-units`), and toggle reference visibility in the active view.

use super::App;
use crate::command;
use crate::tools::{MAX_TEXT, ToolError, decode, rejection, text};
use beamcraft_json::Json;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

/// One attached reference wrapper reported by `inspect-reference`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReferenceRow {
    /// Wrapper element ID.
    pub(crate) id: String,
    /// User label (`bim:name`).
    pub(crate) name: String,
    /// Stored layer name (`external/<sha256>.usda`).
    pub(crate) layer: String,
    /// Source format (`usda` or `usdc`).
    pub(crate) format: String,
    /// Units status (`declared`, `corrected`, `missing`, ...).
    pub(crate) units_status: String,
    /// Number of foreign prims composed under the wrapper.
    pub(crate) prim_count: u64,
    /// Wrapper translation in mm.
    pub(crate) translation_mm: [f64; 3],
}

/// State of the Reference Models bench between frames.
pub(crate) struct Panel {
    /// Source file path for `attach-reference --source`.
    pub(crate) source_path: String,
    /// Optional label for `attach-reference --name`.
    pub(crate) name: String,
    /// Translation `[X, Y, Z]` in mm for attach or move.
    pub(crate) translation: [String; 3],
    /// Plan rotation about Z in degrees (`--rotation-z-deg`).
    pub(crate) rotation_z_deg: String,
    /// Selected reference wrapper ID for move / units correction.
    pub(crate) selected_id: Option<String>,
    /// `--meters-per-unit` for `correct-reference-units`.
    pub(crate) meters_per_unit: String,
    /// `--up-axis` (`Z` or `Y`) for `correct-reference-units`.
    pub(crate) up_axis: String,
    /// Attached reference wrappers in the head revision.
    pub(crate) list: Vec<ReferenceRow>,
    pub(super) list_job: Option<Receiver<Result<Vec<ReferenceRow>, ToolError>>>,
    pub(super) commit_job: Option<Receiver<Result<String, ToolError>>>,
    /// Last outcome or refusal message.
    pub(crate) note: Option<Result<String, String>>,
}

impl Panel {
    pub(super) fn empty() -> Self {
        Self {
            source_path: String::new(),
            name: String::new(),
            translation: ["0".into(), "0".into(), "0".into()],
            rotation_z_deg: "0".into(),
            selected_id: None,
            meters_per_unit: "0.001".into(),
            up_axis: "Z".into(),
            list: Vec::new(),
            list_job: None,
            commit_job: None,
            note: None,
        }
    }

    pub(crate) fn busy(&self) -> bool {
        self.commit_job.is_some()
    }
}

fn parse_triple(parts: &[String; 3]) -> Result<String, String> {
    let mut vals = [0.0f64; 3];
    for (slot, text) in vals.iter_mut().zip(parts) {
        let t = text.trim();
        let v = t
            .parse::<f64>()
            .map_err(|_| format!("Translation: \"{t}\" is not a valid number."))?;
        if !v.is_finite() {
            return Err("Translation: coordinates must be finite numbers.".into());
        }
        *slot = v;
    }
    let [x, y, z] = vals;
    Ok(format!("{x},{y},{z}"))
}

/// Builds the CLI arguments for `attach-reference`.
pub(crate) fn attach_args(
    store: &Path,
    actor: Option<&str>,
    source_path: &str,
    name: &str,
    translation: &[String; 3],
    rotation_z_deg: &str,
) -> Result<Vec<String>, String> {
    let source_path = source_path.trim();
    if source_path.is_empty() {
        return Err("Source file: choose or enter a local USD file path.".into());
    }
    let name = name.trim();
    if name.len() > MAX_TEXT {
        return Err(format!("Name: at most {MAX_TEXT} bytes."));
    }
    let trans = parse_triple(translation)?;
    let rot = rotation_z_deg.trim();
    let mut args = vec![
        "attach-reference".into(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--source".into(),
        source_path.to_owned(),
        "--translation".into(),
        trans,
    ];
    if !name.is_empty() {
        args.extend(["--name".into(), name.to_owned()]);
    }
    if !rot.is_empty() {
        let deg = rot
            .parse::<f64>()
            .map_err(|_| format!("Rotation Z: \"{rot}\" is not a valid number."))?;
        if !deg.is_finite() {
            return Err("Rotation Z: enter a finite number of degrees.".into());
        }
        args.extend(["--rotation-z-deg".into(), rot.to_owned()]);
    }
    if let Some(actor) = actor {
        args.extend(["--actor-id".into(), actor.to_owned()]);
    }
    Ok(args)
}

/// Builds the CLI arguments for `move-reference`.
pub(crate) fn move_args(
    store: &Path,
    actor: Option<&str>,
    id: &str,
    translation: &[String; 3],
    rotation_z_deg: &str,
) -> Result<Vec<String>, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("Select an attached reference to move.".into());
    }
    let trans = parse_triple(translation)?;
    let rot = rotation_z_deg.trim();
    let mut args = vec![
        "move-reference".into(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--id".into(),
        id.to_owned(),
        "--translation".into(),
        trans,
    ];
    if !rot.is_empty() {
        let deg = rot
            .parse::<f64>()
            .map_err(|_| format!("Rotation Z: \"{rot}\" is not a valid number."))?;
        if !deg.is_finite() {
            return Err("Rotation Z: enter a finite number of degrees.".into());
        }
        args.extend(["--rotation-z-deg".into(), rot.to_owned()]);
    }
    if let Some(actor) = actor {
        args.extend(["--actor-id".into(), actor.to_owned()]);
    }
    Ok(args)
}

/// Builds the CLI arguments for `correct-reference-units`.
pub(crate) fn units_args(
    store: &Path,
    actor: Option<&str>,
    id: &str,
    meters_per_unit: &str,
    up_axis: &str,
    clear: bool,
) -> Result<Vec<String>, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("Select an attached reference to update units.".into());
    }
    let mut args = vec![
        "correct-reference-units".into(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--id".into(),
        id.to_owned(),
    ];
    if clear {
        args.extend(["--clear".into(), "true".into()]);
    } else {
        let mpu = meters_per_unit.trim();
        if !mpu.is_empty() {
            let v = mpu
                .parse::<f64>()
                .map_err(|_| format!("Meters per unit: \"{mpu}\" is not a valid number."))?;
            if !v.is_finite() || v <= 0.0 {
                return Err("Meters per unit: enter a positive number.".into());
            }
            args.extend(["--meters-per-unit".into(), mpu.to_owned()]);
        }
        let axis = up_axis.trim();
        if matches!(axis, "Z" | "Y") {
            args.extend(["--up-axis".into(), axis.to_owned()]);
        }
    }
    if let Some(actor) = actor {
        args.extend(["--actor-id".into(), actor.to_owned()]);
    }
    Ok(args)
}

/// Parses the JSON output of `inspect-reference --store ROOT`.
pub(crate) fn parse_references(
    exit_code: Option<i32>,
    stdout: &[u8],
) -> Result<Vec<ReferenceRow>, ToolError> {
    let reply = decode(stdout)?;
    if let Some(error) = rejection(&reply, exit_code) {
        return Err(error);
    }
    let items = reply
        .get("references")
        .and_then(Json::array)
        .map_err(|_| ToolError::local("UNREADABLE_REPLY", "missing references array"))?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let id = text(item, "id")
            .ok_or_else(|| ToolError::local("UNREADABLE_REPLY", "reference lacks id"))?;
        let name = text(item, "name").unwrap_or_default();
        let source = item.get("source").ok();
        let layer = source
            .and_then(|s| text(s, "layer"))
            .unwrap_or_else(|| "external".into());
        let format = source
            .and_then(|s| text(s, "format"))
            .unwrap_or_else(|| "usd".into());
        let units_status = text(item, "units_status").unwrap_or_else(|| "unknown".into());
        let prim_count = item
            .get("foreign_prim_count")
            .and_then(Json::unsigned)
            .unwrap_or(0);
        let translation_mm = item
            .get("placement")
            .and_then(|p| p.get("translation_mm"))
            .and_then(Json::array)
            .ok()
            .and_then(|arr| match arr {
                [x, y, z] => Some([x.float().ok()?, y.float().ok()?, z.float().ok()?]),
                _ => None,
            })
            .unwrap_or([0.0; 3]);
        out.push(ReferenceRow {
            id,
            name,
            layer,
            format,
            units_status,
            prim_count,
            translation_mm,
        });
    }
    Ok(out)
}

impl App {
    /// Starts a read-only `inspect-reference --store ROOT` query on a worker thread.
    pub(crate) fn reference_load(&mut self) {
        if self.source.read_only() || self.reference.list_job.is_some() {
            return;
        }
        let cli = self.source.cli.clone();
        let store = self.source.store.to_string_lossy().into_owned();
        let stats = Arc::clone(&self.stats);
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("beamcraft-references-list".into())
            .spawn(move || {
                let args = vec!["inspect-reference".to_owned(), "--store".into(), store];
                let res = command::run(&cli, &args, command::DEADLINE, &stats)
                    .and_then(|out| parse_references(out.code, &out.stdout));
                let _ = tx.send(res);
            });
        if spawned.is_ok() {
            self.reference.list_job = Some(rx);
        }
    }

    /// Polls background reference listing and commit jobs.
    pub(crate) fn poll_reference(&mut self) {
        if let Some(rx) = &self.reference.list_job {
            match rx.try_recv() {
                Ok(Ok(list)) => {
                    self.reference.list_job = None;
                    if self.reference.selected_id.is_none()
                        && let Some(first) = list.first()
                    {
                        self.reference.selected_id = Some(first.id.clone());
                        self.reference.translation = first.translation_mm.map(|v| format!("{v}"));
                    }
                    self.reference.list = list;
                }
                Ok(Err(err)) => {
                    self.reference.list_job = None;
                    if err.code != "NO_REVISION" {
                        self.reference.note = Some(Err(err.to_string()));
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.reference.list_job = None;
                }
            }
        }
        let Some(rx) = &self.reference.commit_job else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(res) => res,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(ToolError::local(
                "CLI_FAILED",
                "The reference command ended without a result.",
            )),
        };
        self.reference.commit_job = None;
        match result {
            Ok(msg) => {
                self.reference.note = Some(Ok(msg));
                self.reference_load();
                self.reload();
            }
            Err(error) => {
                if error.code == "OUTCOME_UNKNOWN" {
                    self.reference_load();
                    self.reload();
                }
                self.reference.note = Some(Err(error.to_string()));
            }
        }
    }

    fn run_reference_cmd(&mut self, args: Vec<String>, summary: String) {
        if self.other_command_running() || self.reference.busy() {
            self.reference.note = Some(Err("Another command is running; wait for it.".into()));
            return;
        }
        let cli = self.source.cli.clone();
        let stats = Arc::clone(&self.stats);
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("beamcraft-reference-cmd".into())
            .spawn(move || {
                let res = command::run(&cli, &args, command::DEADLINE, &stats).and_then(|out| {
                    let reply = decode(&out.stdout)?;
                    rejection(&reply, out.code).map_or(Ok(summary), Err)
                });
                let _ = tx.send(res);
            });
        match spawned {
            Ok(_) => {
                self.reference.commit_job = Some(rx);
                self.reference.note = None;
            }
            Err(_) => {
                self.reference.note =
                    Some(Err("The reference command could not be started.".into()));
            }
        }
    }

    /// Bottom action bar for `Bench::Reference`.
    pub(crate) fn reference_actions(&mut self, ui: &mut egui::Ui) {
        let busy = self.reference.busy() || self.other_command_running();
        let done = crate::kit::actions(ui, "Attach reference", !busy, "Cancel", busy);
        if done.primary {
            match attach_args(
                &self.source.store,
                self.source.actor.as_deref(),
                &self.reference.source_path,
                &self.reference.name,
                &self.reference.translation,
                &self.reference.rotation_z_deg,
            ) {
                Ok(args) => {
                    self.run_reference_cmd(args, "Read-only USD reference attached.".into())
                }
                Err(msg) => self.reference.note = Some(Err(msg)),
            }
        }
        if done.cancel {
            self.set_bench(super::workbench::Bench::Select);
        }
    }

    /// Renders the USD Reference Models inspector panel.
    pub(crate) fn reference_panel(&mut self, ui: &mut egui::Ui) {
        use crate::kit;
        use crate::theme::{self, Icon, Palette};
        let palette = Palette::current(ui);
        ui.set_max_width(ui.available_width());
        kit::type_header(ui, Icon::Reference, "USD references", |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::icon_button(
                    ui,
                    Icon::Refresh,
                    "Reload references",
                    !self.reference.busy() && self.reference.list_job.is_none(),
                )
                .clicked()
                {
                    self.reference_load();
                }
            });
        });

        let busy = self.reference.busy() || self.other_command_running();

        kit::section(ui, "ref", "Attach USD reference", true, |ui| {
            kit::prop_row(ui, "USD file", |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.reference.source_path)
                            .desired_width((ui.available_width() - 68.0).max(60.0))
                            .hint_text("path/to/model.usda"),
                    );
                    if ui
                        .add_enabled(!busy, egui::Button::new("Browse…").small())
                        .clicked()
                    {
                        let start = self.source.store.parent().filter(|d| d.is_dir());
                        if let Ok(Some(chosen)) =
                            crate::dialog::pick_file("Select a USD reference file", start)
                        {
                            self.reference.source_path = chosen.to_string_lossy().into_owned();
                        }
                    }
                });
            });
            super::field(ui, "Name", &mut self.reference.name, 140.0);
            for (label, coord) in ["X, mm", "Y, mm", "Z, mm"]
                .into_iter()
                .zip(&mut self.reference.translation)
            {
                super::field(ui, label, coord, 110.0);
            }
            super::field(
                ui,
                "Rotation Z, °",
                &mut self.reference.rotation_z_deg,
                110.0,
            );
        });

        kit::section(ui, "ref", "Attached references", true, |ui| {
            if self.reference.list.is_empty() {
                kit::prop_row(ui, "Status", |ui| {
                    ui.label(
                        egui::RichText::new("No USD references attached.")
                            .size(11.0)
                            .color(palette.muted),
                    );
                });
            } else {
                let current_id = self.reference.selected_id.clone().unwrap_or_default();
                let current_label = self
                    .reference
                    .list
                    .iter()
                    .find(|r| r.id == current_id)
                    .map_or_else(
                        || "Select reference…".to_owned(),
                        |r| format!("{} ({} prims)", r.name, r.prim_count),
                    );
                let mut picked_coords = None;
                kit::prop_row(ui, "Reference", |ui| {
                    egui::ComboBox::from_id_salt("reference-select")
                        .width((ui.available_width() - kit::FIT).max(80.0))
                        .truncate()
                        .selected_text(current_label)
                        .show_ui(ui, |ui| {
                            for row in &self.reference.list {
                                let label = format!("{} ({} prims)", row.name, row.prim_count);
                                if ui
                                    .selectable_label(
                                        self.reference.selected_id.as_deref()
                                            == Some(row.id.as_str()),
                                        label,
                                    )
                                    .clicked()
                                {
                                    self.reference.selected_id = Some(row.id.clone());
                                    picked_coords = Some(row.translation_mm);
                                }
                            }
                        });
                });
                if let Some(coords) = picked_coords {
                    self.reference.translation = coords.map(|v| format!("{v}"));
                }

                if let Some(active) = self
                    .reference
                    .list
                    .iter()
                    .find(|r| Some(&r.id) == self.reference.selected_id.as_ref())
                    .cloned()
                {
                    kit::prop_info(ui, "ID", Some(&active.id), "—");
                    kit::prop_info(ui, "Layer", Some(&active.layer), "—");
                    let info = format!(
                        "{} · {} prim(s) · units: {}",
                        active.format, active.prim_count, active.units_status
                    );
                    kit::prop_info(ui, "Source", Some(&info), "—");

                    kit::prop_row(ui, "Placement", |ui| {
                        if ui
                            .add_enabled(!busy, egui::Button::new("Move reference").small())
                            .clicked()
                        {
                            match move_args(
                                &self.source.store,
                                self.source.actor.as_deref(),
                                &active.id,
                                &self.reference.translation,
                                &self.reference.rotation_z_deg,
                            ) {
                                Ok(args) => self
                                    .run_reference_cmd(args, "Reference placement updated.".into()),
                                Err(msg) => self.reference.note = Some(Err(msg)),
                            }
                        }
                    });

                    super::field(
                        ui,
                        "Meters per unit",
                        &mut self.reference.meters_per_unit,
                        90.0,
                    );
                    super::creation_panel::token_combo(
                        ui,
                        "ref-up-axis",
                        "Up axis",
                        &mut self.reference.up_axis,
                        &[("Z", "Z up"), ("Y", "Y up")],
                    );
                    kit::prop_row(ui, "Units", |ui| {
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(!busy, egui::Button::new("Apply units").small())
                                .clicked()
                            {
                                match units_args(
                                    &self.source.store,
                                    self.source.actor.as_deref(),
                                    &active.id,
                                    &self.reference.meters_per_unit,
                                    &self.reference.up_axis,
                                    false,
                                ) {
                                    Ok(args) => self
                                        .run_reference_cmd(args, "Reference units updated.".into()),
                                    Err(msg) => self.reference.note = Some(Err(msg)),
                                }
                            }
                            if ui
                                .add_enabled(!busy, egui::Button::new("Reset units").small())
                                .clicked()
                            {
                                match units_args(
                                    &self.source.store,
                                    self.source.actor.as_deref(),
                                    &active.id,
                                    "",
                                    "",
                                    true,
                                ) {
                                    Ok(args) => self
                                        .run_reference_cmd(args, "Reference units cleared.".into()),
                                    Err(msg) => self.reference.note = Some(Err(msg)),
                                }
                            }
                        });
                    });
                }
            }
        });
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn reference_command_builders_match_cli_contract() {
        let store = Path::new("model");
        let t = ["100".to_owned(), "200".to_owned(), "0".to_owned()];
        assert_eq!(
            attach_args(store, Some("act"), " arch.usda ", "Arch", &t, "90").unwrap(),
            [
                "attach-reference",
                "--store",
                "model",
                "--source",
                "arch.usda",
                "--translation",
                "100,200,0",
                "--name",
                "Arch",
                "--rotation-z-deg",
                "90",
                "--actor-id",
                "act"
            ]
        );
        assert_eq!(
            move_args(store, None, "ref-1", &t, "0").unwrap(),
            [
                "move-reference",
                "--store",
                "model",
                "--id",
                "ref-1",
                "--translation",
                "100,200,0",
                "--rotation-z-deg",
                "0"
            ]
        );
        assert_eq!(
            units_args(store, None, "ref-1", "0.001", "Z", false).unwrap(),
            [
                "correct-reference-units",
                "--store",
                "model",
                "--id",
                "ref-1",
                "--meters-per-unit",
                "0.001",
                "--up-axis",
                "Z"
            ]
        );
    }

    #[test]
    fn parse_references_reads_inspection_json() {
        let json = br#"{"outcome":"reference_inspection","reference_count":1,"references":[{"id":"019a7bf0-0000-7000-8000-000000000011","name":"Arch","placement":{"translation_mm":[10.0,20.0,30.0]},"source":{"layer":"external/abc.usda","format":"usda"},"units_status":"declared","foreign_prim_count":42}]}"#;
        let rows = parse_references(Some(0), json).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Arch");
        assert_eq!(rows[0].prim_count, 42);
        assert_eq!(rows[0].translation_mm, [10.0, 20.0, 30.0]);
    }
}
