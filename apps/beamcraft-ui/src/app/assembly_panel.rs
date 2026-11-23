// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The Assemblies bench (#844): list the model's assemblies (`assemblies`), create a new
//! assembly (`create-assembly`), set its main part (`set-main-part`), attach or detach parts
//! (`attach-to-assembly`, `detach-from-assembly`) and dissolve an assembly into loose parts
//! (`explode-assembly`). Every action runs the CLI command on a worker thread and reloads the
//! scene when it finishes.

use super::App;
use crate::command;
use crate::edit::{Ray, pick_in};
use crate::tools::{MAX_TEXT, ToolError, decode, rejection, text};
use beamcraft_json::Json;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

/// One assembly reported by `assemblies --store ROOT`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AssemblyRow {
    /// Assembly identity (UUIDv7).
    pub(crate) id: String,
    /// User label (`bim:name`).
    pub(crate) name: String,
    /// Parent assembly when nested (`part_of`).
    pub(crate) parent: Option<String>,
    /// Main part identity (`main`).
    pub(crate) main: Option<String>,
    /// Direct member part identities (`parts`).
    pub(crate) parts: Vec<String>,
}

/// State of the Assemblies bench between frames.
pub(crate) struct Panel {
    /// Name for a newly created assembly (`--name`).
    pub(crate) name: String,
    /// Main part picked for a new assembly or for `set-main-part`.
    pub(crate) main_part: Option<String>,
    /// Additional parts picked to attach to the assembly.
    pub(crate) parts: Vec<String>,
    /// Selected existing assembly identity in the model list.
    pub(crate) selected_assembly: Option<String>,
    /// Target part identity for attach/detach/set-main on the selected assembly.
    pub(crate) target_part: String,
    /// Assemblies loaded from the store head.
    pub(crate) list: Vec<AssemblyRow>,
    pub(super) list_job: Option<Receiver<Result<Vec<AssemblyRow>, ToolError>>>,
    pub(super) commit_job: Option<Receiver<Result<String, ToolError>>>,
    /// Last outcome or refusal message.
    pub(crate) note: Option<Result<String, String>>,
}

impl Panel {
    pub(super) fn empty() -> Self {
        Self {
            name: "Assembly 1".into(),
            main_part: None,
            parts: Vec::new(),
            selected_assembly: None,
            target_part: String::new(),
            list: Vec::new(),
            list_job: None,
            commit_job: None,
            note: None,
        }
    }

    pub(crate) fn busy(&self) -> bool {
        self.commit_job.is_some()
    }

    pub(crate) fn clear_picks(&mut self) {
        self.main_part = None;
        self.parts.clear();
        self.target_part.clear();
    }
}

/// Builds the CLI arguments for `create-assembly`.
pub(crate) fn create_args(
    store: &Path,
    actor: Option<&str>,
    name: &str,
    new_id: Option<&str>,
) -> Result<Vec<String>, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Name: enter a label for the assembly.".into());
    }
    if name.len() > MAX_TEXT {
        return Err(format!("Name: at most {MAX_TEXT} bytes."));
    }
    let mut args = vec![
        "create-assembly".into(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--name".into(),
        name.to_owned(),
    ];
    if let Some(id) = new_id.map(str::trim).filter(|s| !s.is_empty()) {
        args.extend(["--new-id".into(), id.to_owned()]);
    }
    if let Some(actor) = actor {
        args.extend(["--actor-id".into(), actor.to_owned()]);
    }
    Ok(args)
}

/// Builds the CLI arguments for `attach-to-assembly`.
pub(crate) fn attach_args(
    store: &Path,
    actor: Option<&str>,
    assembly_id: &str,
    part_id: &str,
) -> Result<Vec<String>, String> {
    let assembly_id = assembly_id.trim();
    let part_id = part_id.trim();
    if assembly_id.is_empty() || part_id.is_empty() {
        return Err("Select an assembly and pick a part in the view.".into());
    }
    let mut args = vec![
        "attach-to-assembly".into(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--assembly-id".into(),
        assembly_id.to_owned(),
        "--id".into(),
        part_id.to_owned(),
    ];
    if let Some(actor) = actor {
        args.extend(["--actor-id".into(), actor.to_owned()]);
    }
    Ok(args)
}

/// Builds the CLI arguments for `detach-from-assembly`.
pub(crate) fn detach_args(
    store: &Path,
    actor: Option<&str>,
    part_id: &str,
) -> Result<Vec<String>, String> {
    let part_id = part_id.trim();
    if part_id.is_empty() {
        return Err("Pick or select the part to detach.".into());
    }
    let mut args = vec![
        "detach-from-assembly".into(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--id".into(),
        part_id.to_owned(),
    ];
    if let Some(actor) = actor {
        args.extend(["--actor-id".into(), actor.to_owned()]);
    }
    Ok(args)
}

/// Builds the CLI arguments for `set-main-part`.
pub(crate) fn set_main_args(
    store: &Path,
    actor: Option<&str>,
    assembly_id: &str,
    part_id: &str,
) -> Result<Vec<String>, String> {
    let assembly_id = assembly_id.trim();
    let part_id = part_id.trim();
    if assembly_id.is_empty() || part_id.is_empty() {
        return Err("Select an assembly and pick the main part.".into());
    }
    let mut args = vec![
        "set-main-part".into(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--assembly-id".into(),
        assembly_id.to_owned(),
        "--id".into(),
        part_id.to_owned(),
    ];
    if let Some(actor) = actor {
        args.extend(["--actor-id".into(), actor.to_owned()]);
    }
    Ok(args)
}

/// Builds the CLI arguments for `explode-assembly`.
pub(crate) fn explode_args(
    store: &Path,
    actor: Option<&str>,
    assembly_id: &str,
) -> Result<Vec<String>, String> {
    let assembly_id = assembly_id.trim();
    if assembly_id.is_empty() {
        return Err("Select an assembly to explode.".into());
    }
    let mut args = vec![
        "explode-assembly".into(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--id".into(),
        assembly_id.to_owned(),
    ];
    if let Some(actor) = actor {
        args.extend(["--actor-id".into(), actor.to_owned()]);
    }
    Ok(args)
}

/// Parses the JSON output of `assemblies --store ROOT`.
pub(crate) fn parse_assemblies(
    exit_code: Option<i32>,
    stdout: &[u8],
) -> Result<Vec<AssemblyRow>, ToolError> {
    let reply = decode(stdout)?;
    if let Some(error) = rejection(&reply, exit_code) {
        return Err(error);
    }
    let items = reply
        .get("assemblies")
        .and_then(Json::array)
        .map_err(|_| ToolError::local("UNREADABLE_REPLY", "missing assemblies array"))?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let id = text(item, "id")
            .ok_or_else(|| ToolError::local("UNREADABLE_REPLY", "assembly lacks id"))?;
        let name = text(item, "name").unwrap_or_default();
        let parent = text(item, "part_of");
        let main = text(item, "main");
        let parts = item
            .get("parts")
            .and_then(Json::array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.string().ok().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        out.push(AssemblyRow {
            id,
            name,
            parent,
            main,
            parts,
        });
    }
    Ok(out)
}

impl App {
    /// Starts a read-only `assemblies --store ROOT` query on a worker thread.
    pub(crate) fn assembly_load(&mut self) {
        if self.source.read_only() || self.assembly.list_job.is_some() {
            return;
        }
        let cli = self.source.cli.clone();
        let store = self.source.store.to_string_lossy().into_owned();
        let stats = Arc::clone(&self.stats);
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("beamcraft-assemblies-list".into())
            .spawn(move || {
                let args = vec!["assemblies".to_owned(), "--store".into(), store];
                let res = command::run(&cli, &args, command::DEADLINE, &stats)
                    .and_then(|out| parse_assemblies(out.code, &out.stdout));
                let _ = tx.send(res);
            });
        if spawned.is_ok() {
            self.assembly.list_job = Some(rx);
        }
    }

    /// Polls the background `assemblies` listing and any running assembly command.
    pub(crate) fn poll_assembly(&mut self) {
        if let Some(rx) = &self.assembly.list_job {
            match rx.try_recv() {
                Ok(Ok(list)) => {
                    self.assembly.list_job = None;
                    if self.assembly.selected_assembly.is_none() {
                        self.assembly.selected_assembly = list.first().map(|a| a.id.clone());
                    } else if let Some(sel) = &self.assembly.selected_assembly
                        && !list.iter().any(|a| &a.id == sel)
                    {
                        self.assembly.selected_assembly = list.first().map(|a| a.id.clone());
                    }
                    self.assembly.list = list;
                }
                Ok(Err(err)) => {
                    self.assembly.list_job = None;
                    self.assembly.note = Some(Err(err.to_string()));
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.assembly.list_job = None;
                }
            }
        }
        let Some(rx) = &self.assembly.commit_job else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(res) => res,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(ToolError::local(
                "CLI_FAILED",
                "The assembly command ended without a result.",
            )),
        };
        self.assembly.commit_job = None;
        match result {
            Ok(msg) => {
                self.assembly.note = Some(Ok(msg));
                self.assembly_load();
                self.reload();
            }
            Err(error) => {
                if error.code == "OUTCOME_UNKNOWN" {
                    self.assembly_load();
                    self.reload();
                }
                self.assembly.note = Some(Err(error.to_string()));
            }
        }
    }

    /// Handles a click in the viewport while on `Bench::Assembly`.
    pub(crate) fn assembly_click(&mut self, ray: Ray) {
        if self.other_command_running() || self.assembly.busy() {
            return;
        }
        let Some(loaded) = &self.loaded else {
            return;
        };
        let picked = match self.views.active_view() {
            Some(view) => beamcraft_engine::views::pick_visible(
                &loaded.snapshot,
                view,
                ray.origin,
                ray.direction,
                self.selection.filter(),
                false,
            )
            .map(|hit| hit.map(|part| part.element_id.to_string()))
            .map_err(|e| ToolError::local("PICK_REFUSED", e.to_string())),
            None => pick_in(&loaded.snapshot, &ray, self.selection.filter()),
        };
        match picked {
            Ok(Some(id)) => {
                self.assembly.target_part = id.clone();
                if self.assembly.main_part.is_none() {
                    self.assembly.main_part = Some(id);
                    self.assembly.note = Some(Ok(
                        "Main part picked. Click secondary parts to add, or Create assembly."
                            .into(),
                    ));
                } else if self.assembly.main_part.as_deref() != Some(id.as_str()) {
                    if let Some(pos) = self.assembly.parts.iter().position(|p| p == &id) {
                        self.assembly.parts.remove(pos);
                    } else {
                        self.assembly.parts.push(id);
                    }
                    self.assembly.note = Some(Ok(format!(
                        "Main part + {} secondary part(s) picked.",
                        self.assembly.parts.len()
                    )));
                }
            }
            Ok(None) => {
                self.assembly.note = Some(Err("No part was hit; click a model part.".into()));
            }
            Err(error) => self.assembly.note = Some(Err(error.to_string())),
        }
    }

    fn run_assembly_batch(&mut self, steps: Vec<Vec<String>>, summary: String) {
        if self.other_command_running() || self.assembly.busy() {
            self.assembly.note = Some(Err("Another command is running; wait for it.".into()));
            return;
        }
        let cli = self.source.cli.clone();
        let stats = Arc::clone(&self.stats);
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("beamcraft-assembly-cmd".into())
            .spawn(move || {
                for args in steps {
                    let result =
                        command::run(&cli, &args, command::DEADLINE, &stats).and_then(|out| {
                            let reply = decode(&out.stdout)?;
                            rejection(&reply, out.code).map_or(Ok(()), Err)
                        });
                    if let Err(err) = result {
                        let _ = tx.send(Err(err));
                        return;
                    }
                }
                let _ = tx.send(Ok(summary));
            });
        match spawned {
            Ok(_) => {
                self.assembly.commit_job = Some(rx);
                self.assembly.note = None;
            }
            Err(_) => {
                self.assembly.note = Some(Err("The assembly command could not be started.".into()));
            }
        }
    }

    /// Bottom action bar for `Bench::Assembly`.
    pub(crate) fn assembly_actions(&mut self, ui: &mut egui::Ui) {
        let busy = self.assembly.busy() || self.other_command_running();
        let done = crate::kit::actions(ui, "Create assembly", !busy, "Cancel", busy);
        if done.primary {
            self.start_create_assembly();
        }
        if done.cancel {
            self.assembly.clear_picks();
            self.set_bench(super::workbench::Bench::Select);
        }
    }

    fn start_create_assembly(&mut self) {
        let new_id = crate::sheet::fresh_id();
        match create_args(
            &self.source.store,
            self.source.actor.as_deref(),
            &self.assembly.name,
            new_id.as_deref(),
        ) {
            Ok(create) => {
                let mut steps = vec![create];
                if let Some(asm_id) = new_id.as_deref() {
                    if let Some(main) = self.assembly.main_part.clone() {
                        if let Ok(att) = attach_args(
                            &self.source.store,
                            self.source.actor.as_deref(),
                            asm_id,
                            &main,
                        ) {
                            steps.push(att);
                        }
                        if let Ok(set_m) = set_main_args(
                            &self.source.store,
                            self.source.actor.as_deref(),
                            asm_id,
                            &main,
                        ) {
                            steps.push(set_m);
                        }
                    }
                    for sec in &self.assembly.parts {
                        if let Ok(att) = attach_args(
                            &self.source.store,
                            self.source.actor.as_deref(),
                            asm_id,
                            sec,
                        ) {
                            steps.push(att);
                        }
                    }
                    self.assembly.selected_assembly = Some(asm_id.to_owned());
                }
                let label = self.assembly.name.trim().to_owned();
                self.assembly.clear_picks();
                self.run_assembly_batch(steps, format!("Assembly \"{label}\" created."));
            }
            Err(msg) => self.assembly.note = Some(Err(msg)),
        }
    }

    /// Renders the Assemblies inspector panel.
    pub(crate) fn assembly_panel(&mut self, ui: &mut egui::Ui) {
        use crate::kit;
        use crate::theme::{self, Icon, Kind, Palette};
        let palette = Palette::current(ui);
        ui.set_max_width(ui.available_width());
        kit::type_header(ui, Icon::Assembly, "Assemblies", |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::icon_button(
                    ui,
                    Icon::Refresh,
                    "Reload assemblies",
                    !self.assembly.busy() && self.assembly.list_job.is_none(),
                )
                .clicked()
                {
                    self.assembly_load();
                }
            });
        });

        let busy = self.assembly.busy() || self.other_command_running();
        let selected_element = self.editor.selected().map(|s| s.id.clone());

        kit::section(ui, "asm", "Create assembly", true, |ui| {
            super::field(ui, "Name", &mut self.assembly.name, 140.0);
            kit::prop_info(
                ui,
                "1. Main part",
                self.assembly.main_part.as_deref(),
                "Click main part in view",
            );
            let count_text = if self.assembly.parts.is_empty() {
                None
            } else {
                Some(format!("{} picked", self.assembly.parts.len()))
            };
            kit::prop_info(
                ui,
                "2. Secondary",
                count_text.as_deref(),
                "Click parts in view",
            );
            kit::prop_row(ui, "", |ui| {
                ui.horizontal(|ui| {
                    if let Some(sel) = selected_element.as_deref()
                        && ui
                            .add_enabled(!busy, egui::Button::new("Use selected").small())
                            .clicked()
                    {
                        self.assembly.main_part = Some(sel.to_owned());
                        self.assembly.target_part = sel.to_owned();
                    }
                    if ui
                        .add_enabled(
                            !busy
                                && (self.assembly.main_part.is_some()
                                    || !self.assembly.parts.is_empty()),
                            egui::Button::new("Clear picks").small(),
                        )
                        .clicked()
                    {
                        self.assembly.clear_picks();
                    }
                });
            });
        });

        kit::section(ui, "asm", "Numbering series", true, |ui| {
            super::field(
                ui,
                "Assembly prefix",
                &mut self.numbering.assembly_prefix,
                80.0,
            );
            super::field(ui, "Start number", &mut self.numbering.assembly_start, 80.0);
        });

        kit::section(ui, "asm", "Assemblies in model", true, |ui| {
            if self.assembly.list.is_empty() {
                kit::prop_row(ui, "Status", |ui| {
                    ui.label(
                        egui::RichText::new("No assemblies in model yet.")
                            .size(11.0)
                            .color(palette.muted),
                    );
                });
            } else {
                let current_id = self.assembly.selected_assembly.clone().unwrap_or_default();
                let current_label = self
                    .assembly
                    .list
                    .iter()
                    .find(|a| a.id == current_id)
                    .map_or_else(
                        || "Select assembly…".to_owned(),
                        |a| format!("{} ({} parts)", a.name, a.parts.len()),
                    );
                kit::prop_row(ui, "Assembly", |ui| {
                    egui::ComboBox::from_id_salt("assembly-select")
                        .width((ui.available_width() - kit::FIT).max(80.0))
                        .truncate()
                        .selected_text(current_label)
                        .show_ui(ui, |ui| {
                            for row in &self.assembly.list {
                                let text = format!("{} ({} parts)", row.name, row.parts.len());
                                ui.selectable_value(
                                    &mut self.assembly.selected_assembly,
                                    Some(row.id.clone()),
                                    text,
                                );
                            }
                        });
                });

                if let Some(active) = self
                    .assembly
                    .list
                    .iter()
                    .find(|a| Some(&a.id) == self.assembly.selected_assembly.as_ref())
                    .cloned()
                {
                    kit::prop_info(ui, "ID", Some(&active.id), "—");
                    kit::prop_info(ui, "Main part", active.main.as_deref(), "Not set");
                    let members_summary = format!("{} part(s)", active.parts.len());
                    kit::prop_info(ui, "Parts", Some(&members_summary), "0");
                    if !active.parts.is_empty() {
                        kit::prop_row(ui, "", |ui| {
                            if ui.button("Highlight in view").clicked() {
                                self.select_assembly_row(&active);
                            }
                        });
                    }

                    if self.assembly.target_part.is_empty()
                        && let Some(sel) = selected_element.as_deref()
                    {
                        self.assembly.target_part = sel.to_owned();
                    }
                    super::field(ui, "Part ID", &mut self.assembly.target_part, 140.0);

                    kit::prop_row(ui, "Membership", |ui| {
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(
                                    !busy && !self.assembly.target_part.trim().is_empty(),
                                    egui::Button::new("Attach").small(),
                                )
                                .clicked()
                            {
                                match attach_args(
                                    &self.source.store,
                                    self.source.actor.as_deref(),
                                    &active.id,
                                    &self.assembly.target_part,
                                ) {
                                    Ok(args) => self.run_assembly_batch(
                                        vec![args],
                                        "Part attached to assembly.".into(),
                                    ),
                                    Err(msg) => self.assembly.note = Some(Err(msg)),
                                }
                            }
                            if ui
                                .add_enabled(
                                    !busy && !self.assembly.target_part.trim().is_empty(),
                                    egui::Button::new("Set main").small(),
                                )
                                .clicked()
                            {
                                match set_main_args(
                                    &self.source.store,
                                    self.source.actor.as_deref(),
                                    &active.id,
                                    &self.assembly.target_part,
                                ) {
                                    Ok(args) => self.run_assembly_batch(
                                        vec![args],
                                        "Assembly main part updated.".into(),
                                    ),
                                    Err(msg) => self.assembly.note = Some(Err(msg)),
                                }
                            }
                            if ui
                                .add_enabled(
                                    !busy && !self.assembly.target_part.trim().is_empty(),
                                    egui::Button::new("Detach").small(),
                                )
                                .clicked()
                            {
                                match detach_args(
                                    &self.source.store,
                                    self.source.actor.as_deref(),
                                    &self.assembly.target_part,
                                ) {
                                    Ok(args) => self.run_assembly_batch(
                                        vec![args],
                                        "Part detached from assembly.".into(),
                                    ),
                                    Err(msg) => self.assembly.note = Some(Err(msg)),
                                }
                            }
                        });
                    });

                    kit::prop_row(ui, "Dissolve", |ui| {
                        if theme::button(ui, Kind::Secondary, "Explode assembly", !busy).clicked() {
                            match explode_args(
                                &self.source.store,
                                self.source.actor.as_deref(),
                                &active.id,
                            ) {
                                Ok(args) => self.run_assembly_batch(
                                    vec![args],
                                    format!("Assembly \"{}\" exploded.", active.name),
                                ),
                                Err(msg) => self.assembly.note = Some(Err(msg)),
                            }
                        }
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
    fn assembly_command_builders_match_cli_contract() {
        let store = Path::new("model");
        assert_eq!(
            create_args(store, Some("act"), " Truss 1 ", Some("id-1")).unwrap(),
            [
                "create-assembly",
                "--store",
                "model",
                "--name",
                "Truss 1",
                "--new-id",
                "id-1",
                "--actor-id",
                "act"
            ]
        );
        assert_eq!(
            attach_args(store, None, "asm-1", "prt-1").unwrap(),
            [
                "attach-to-assembly",
                "--store",
                "model",
                "--assembly-id",
                "asm-1",
                "--id",
                "prt-1"
            ]
        );
        assert_eq!(
            set_main_args(store, None, "asm-1", "prt-1").unwrap(),
            [
                "set-main-part",
                "--store",
                "model",
                "--assembly-id",
                "asm-1",
                "--id",
                "prt-1"
            ]
        );
        assert_eq!(
            detach_args(store, None, "prt-1").unwrap(),
            ["detach-from-assembly", "--store", "model", "--id", "prt-1"]
        );
        assert_eq!(
            explode_args(store, None, "asm-1").unwrap(),
            ["explode-assembly", "--store", "model", "--id", "asm-1"]
        );
    }

    #[test]
    fn parse_assemblies_reads_views_array() {
        let json = br#"{"outcome":"assemblies","assemblies":[{"id":"019a7bf0-0000-7000-8000-000000000001","name":"A1","part_of":null,"parts":["019a7bf0-0000-7000-8000-000000000002"],"main":"019a7bf0-0000-7000-8000-000000000002"}]}"#;
        let rows = parse_assemblies(Some(0), json).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "A1");
        assert_eq!(rows[0].parts.len(), 1);
        assert_eq!(
            rows[0].main.as_deref(),
            Some("019a7bf0-0000-7000-8000-000000000002")
        );
    }
}
