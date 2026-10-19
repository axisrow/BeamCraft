// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The eframe window: a side panel with the revision, counts, units and adapter, and the viewport
//! with orbit (primary drag), pan (secondary drag), zoom (wheel), fit (F) and reload (R).

use crate::camera::Camera;
use crate::render::{Renderer, Uploaded};
use crate::scene::Scene;
use crate::source::Reloaded;
use crate::tools::{
    Choice, EditResult, ElementRow, MAX_TEXT, MemberKind, ToolError, ToolKind, Tools,
};
use crate::{Error, check_font, command, source};
use beamcraft_engine::ViewSnapshot;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Instant;

/// Where the window reads its model from.
#[derive(Debug, Clone)]
pub struct Source {
    /// The published store.
    pub store: PathBuf,
    /// The `beamcraft` CLI that exports the snapshot.
    pub cli: PathBuf,
}

struct Loaded {
    snapshot: ViewSnapshot,
    scene: Scene,
    uploaded: Uploaded,
}

/// The window state.
pub struct App {
    state: eframe::egui_wgpu::RenderState,
    source: Source,
    renderer: Renderer,
    texture: egui::TextureId,
    camera: Camera,
    loaded: Option<Loaded>,
    /// The rows of the loaded snapshot's elements, cached once per accepted revision so the
    /// panel does not rebuild them every frame.
    elements: Vec<ElementRow>,
    error: Option<String>,
    text: bool,
    adapter: String,
    last: Instant,
    fps: f32,
    tools: Tools,
    job: Option<Receiver<Result<EditResult, ToolError>>>,
    reload_job: Option<Receiver<Reloaded>>,
    reload_queued: bool,
    /// The result of the last Preview, Cancel or Commit.
    tool_note: Option<Result<String, String>>,
    /// Why the last definitions refresh failed; kept apart from `tool_note` so neither hides the other.
    refresh_error: Option<String>,
}

const RED: egui::Color32 = egui::Color32::from_rgb(230, 80, 60);

/// A drop-down of store definitions; `required` removes the "none" entry.
fn choice_combo(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    current: &mut Option<String>,
    choices: &[Choice],
    required: bool,
) {
    let selected = current
        .as_ref()
        .and_then(|d| choices.iter().find(|c| &c.digest == d))
        .map_or_else(
            || if required { "choose…" } else { "none" }.to_owned(),
            |c| c.label.clone(),
        );
    egui::ComboBox::from_id_salt(id)
        .width(200.0)
        .selected_text(selected)
        .show_ui(ui, |ui| {
            if !required {
                ui.selectable_value(current, None, "none");
            }
            for choice in choices {
                ui.selectable_value(current, Some(choice.digest.clone()), &choice.label);
            }
        });
    ui.label(label);
}

fn field(ui: &mut egui::Ui, label: &str, text: &mut String, width: f32) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(
            egui::TextEdit::singleline(text)
                .desired_width(width)
                .char_limit(MAX_TEXT),
        );
    });
}

/// Inter from the admitted asset directory, validated before egui can panic on it.
fn inter(ctx: &egui::Context) -> bool {
    let candidates = [
        Path::new("assets/fonts/inter/Inter-Regular.ttf").to_path_buf(),
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts/inter/Inter-Regular.ttf"),
    ];
    let bytes = candidates.iter().find(|p| p.is_file()).map(|path| {
        use std::io::Read;
        let file = std::fs::File::open(path)?;
        let mut bytes = Vec::new();
        file.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.is_empty() || bytes.len() > 4 * 1024 * 1024 {
            return Err(std::io::Error::other("Inter size limit"));
        }
        check_font(&bytes).map_err(std::io::Error::other)?;
        Ok(bytes)
    });
    match bytes {
        Some(Ok(bytes)) => {
            let mut fonts = egui::FontDefinitions::empty();
            fonts
                .font_data
                .insert("Inter".into(), egui::FontData::from_owned(bytes).into());
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.insert(family, vec!["Inter".into()]);
            }
            ctx.set_fonts(fonts);
            true
        }
        _ => {
            eprintln!("Inter is unavailable; the window runs without text.");
            ctx.set_fonts(egui::FontDefinitions::empty());
            false
        }
    }
}

/// A contributor-original 32 x 32 window icon (an I-section silhouette), drawn in code. It
/// replaces eframe's built-in logo, which is not a BeamCraft mark.
pub fn icon() -> egui::IconData {
    let mut rgba = Vec::with_capacity(32 * 32 * 4);
    for y in 0..32u32 {
        for x in 0..32u32 {
            let flange = (4..10).contains(&y) || (22..28).contains(&y);
            let web = (13..19).contains(&x) && (10..22).contains(&y);
            let inside = (4..28).contains(&x) && (flange || web);
            rgba.extend_from_slice(if inside {
                &[46, 140, 209, 255]
            } else {
                &[0, 0, 0, 0]
            });
        }
    }
    egui::IconData {
        rgba,
        width: 32,
        height: 32,
    }
}

impl App {
    /// Creates the window state and loads the accepted snapshot.
    pub fn new(cc: &eframe::CreationContext<'_>, source: Source) -> Result<Self, Error> {
        let state = cc
            .wgpu_render_state
            .clone()
            .ok_or(Error::Input("eframe requires wgpu"))?;
        let text = inter(&cc.egui_ctx);
        let renderer = Renderer::new(&state.adapter, &state.device, 1280, 800)?;
        let texture = state.renderer.write().register_native_texture(
            &state.device,
            &renderer.view,
            wgpu::FilterMode::Linear,
        );
        let info = state.adapter.get_info();
        let adapter = format!(
            "{} / {:?} / {:?}",
            info.name, info.device_type, info.backend
        );
        let mut app = Self {
            state,
            source,
            renderer,
            texture,
            camera: Camera::new([0.0; 3], 1000.0, 1.6)?,
            loaded: None,
            elements: Vec::new(),
            error: None,
            text,
            adapter,
            last: Instant::now(),
            fps: 0.0,
            tools: Tools::default(),
            job: None,
            reload_job: None,
            reload_queued: false,
            tool_note: None,
            refresh_error: None,
        };
        app.reload();
        Ok(app)
    }

    /// Starts a refresh on a worker thread (the CLI queries and the scene build); the result is
    /// applied by `poll_reload`. A refresh asked for while one runs is queued, so it still reads
    /// the store after whatever just changed it.
    fn reload(&mut self) {
        if self.reload_job.is_some() {
            self.reload_queued = true;
            return;
        }
        let (cli, store) = (self.source.cli.clone(), self.source.store.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("beamcraft-reload".into())
            .spawn(move || {
                let _ = tx.send(source::reload(&cli, &store));
            });
        match spawned {
            Ok(_) => self.reload_job = Some(rx),
            Err(_) => self.error = Some("the reload thread could not be started".into()),
        }
    }

    fn poll_reload(&mut self) {
        let Some(rx) = &self.reload_job else { return };
        let reloaded = match rx.try_recv() {
            Ok(reloaded) => reloaded,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.reload_job = None;
                self.error = Some("the reload thread ended without a result".into());
                return;
            }
        };
        self.reload_job = None;
        self.refresh_error = self.tools.apply_definitions(reloaded.definitions);
        let result = reloaded.scene.and_then(|(snapshot, scene)| {
            let uploaded = Uploaded::new(&self.state.device, &self.state.queue, &scene)?;
            Ok(Loaded {
                snapshot,
                scene,
                uploaded,
            })
        });
        match result {
            Ok(loaded) => {
                let (target, extent) = loaded.scene.framing();
                // A scene the camera cannot frame keeps the previous scene and shows why.
                match self.camera.fit(target, extent, self.aspect()) {
                    Ok(()) => {
                        // A selection the accepted revision no longer lists is dropped; the
                        // rows the panel shows are rebuilt from the same snapshot.
                        self.tools.retain_selection(|id| {
                            loaded
                                .snapshot
                                .elements()
                                .iter()
                                .any(|e| e.id.to_string() == id)
                        });
                        self.elements = loaded
                            .snapshot
                            .elements()
                            .iter()
                            .map(|e| ElementRow {
                                id: e.id.to_string(),
                                kind: e.kind.as_token().to_owned(),
                                placement_mm: e.placement_mm,
                            })
                            .collect();
                        self.loaded = Some(loaded);
                        self.error = None;
                    }
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
            Err(e) => self.error = Some(e.to_string()),
        }
        if std::mem::take(&mut self.reload_queued) {
            self.reload();
        }
    }
    fn aspect(&self) -> f32 {
        (self.renderer.width as f32 / self.renderer.height as f32).clamp(0.05, 20.0)
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), Error> {
        if width == self.renderer.width && height == self.renderer.height {
            return Ok(());
        }
        let renderer = Renderer::new(&self.state.adapter, &self.state.device, width, height)?;
        self.state
            .renderer
            .write()
            .update_egui_texture_from_wgpu_texture(
                &self.state.device,
                &renderer.view,
                wgpu::FilterMode::Linear,
                self.texture,
            );
        self.renderer = renderer;
        Ok(())
    }

    fn panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            self.info(ui);
            ui.separator();
            self.tools_panel(ui);
        });
    }

    fn start_commit(&mut self) {
        let Some(prepared) = self.tools.begin_commit(&self.source.store) else {
            return;
        };
        let cli = self.source.cli.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.job = Some(rx);
        self.tool_note = None;
        let spawned = std::thread::Builder::new()
            .name("beamcraft-command".into())
            .spawn(move || {
                let _ = tx.send(command::run_tool(&cli, &prepared));
            });
        if spawned.is_err() {
            self.job = None;
            self.tools.finish();
            self.tool_note = Some(Err("the command thread could not be started".into()));
        }
    }

    fn poll_commit(&mut self) {
        let Some(rx) = &self.job else { return };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(ToolError {
                code: "CLI_FAILED".into(),
                message: "the command thread ended without a result".into(),
            }),
        };
        self.job = None;
        self.tools.finish();
        match result {
            Ok(result) => {
                // A committed delete removes the selection with its element, even before the
                // reload confirms it.
                if let EditResult::Deleted(deleted) = &result {
                    self.tools.retain_selection(|id| id != deleted.element_id);
                }
                self.reload();
                self.tool_note = Some(Ok(result.summary()));
            }
            Err(e) => {
                if e.code == "OUTCOME_UNKNOWN" {
                    self.reload();
                }
                self.tool_note = Some(Err(e.to_string()));
            }
        }
    }

    fn selection_line(&self, ui: &mut egui::Ui) {
        let selected = self
            .tools
            .selection
            .as_deref()
            .and_then(|id| self.elements.iter().find(|r| r.id == id));
        match selected {
            Some(s) => ui.label(format!(
                "Selected: {} {} at {},{},{} mm.",
                s.kind, s.id, s.placement_mm[0], s.placement_mm[1], s.placement_mm[2]
            )),
            None => ui.colored_label(RED, "No element selected: click one in the Elements list."),
        };
    }

    fn tools_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Elements");
        if self.elements.is_empty() {
            ui.label(if self.loaded.is_some() {
                "The accepted revision has no elements yet."
            } else {
                "No model loaded."
            });
        }
        let (elements, tools) = (&self.elements, &mut self.tools);
        let row_height = ui.text_style_height(&egui::TextStyle::Body);
        egui::ScrollArea::vertical()
            .id_salt("elements")
            .max_height(160.0)
            .show_rows(ui, row_height, elements.len(), |ui, range| {
                for row in elements.get(range).into_iter().flatten() {
                    let selected = tools.selection.as_deref() == Some(row.id.as_str());
                    if ui
                        .selectable_label(selected, format!("{} {}", row.kind, row.id))
                        .clicked()
                    {
                        tools.select(row);
                    }
                }
            });
        ui.separator();
        ui.heading("Create or edit");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tools.active, ToolKind::Plate, "Plate");
            ui.selectable_value(&mut self.tools.active, ToolKind::Member, "Member");
            ui.selectable_value(&mut self.tools.active, ToolKind::Move, "Move");
            ui.selectable_value(&mut self.tools.active, ToolKind::Delete, "Delete");
        });
        ui.label("Dimensions in mm.");
        let materials = self.tools.definitions.materials.clone();
        let profiles = self.tools.definitions.profiles.clone();
        match self.tools.active {
            ToolKind::Plate => {
                let form = &mut self.tools.plate;
                field(ui, "Width", &mut form.width, 80.0);
                field(ui, "Length", &mut form.length, 80.0);
                field(ui, "Thickness", &mut form.thickness, 80.0);
                field(ui, "Name", &mut form.name, 140.0);
                choice_combo(
                    ui,
                    "plate-material",
                    "Material",
                    &mut form.material,
                    &materials,
                    false,
                );
            }
            ToolKind::Member => {
                let form = &mut self.tools.member;
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut form.kind, MemberKind::Beam, "Beam");
                    ui.selectable_value(&mut form.kind, MemberKind::Column, "Column");
                });
                for (label, point) in [("Start", &mut form.start), ("End", &mut form.end)] {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        for (axis, text) in ["X", "Y", "Z"].iter().zip(point.iter_mut()) {
                            ui.label(*axis);
                            ui.add(
                                egui::TextEdit::singleline(text)
                                    .desired_width(56.0)
                                    .char_limit(MAX_TEXT),
                            );
                        }
                    });
                }
                field(ui, "Name", &mut form.name, 140.0);
                choice_combo(
                    ui,
                    "member-profile",
                    "Profile",
                    &mut form.profile,
                    &profiles,
                    true,
                );
                choice_combo(
                    ui,
                    "member-material",
                    "Material",
                    &mut form.material,
                    &materials,
                    false,
                );
                if profiles.is_empty() {
                    ui.label("The store has no profile definition yet (beamcraft define-profile).");
                }
            }
            ToolKind::Move => {
                self.selection_line(ui);
                ui.label("New placement translation (absolute; rotation kept).");
                let placement = self
                    .tools
                    .selection
                    .as_deref()
                    .and_then(|id| self.elements.iter().find(|r| r.id == id))
                    .map(|r| r.placement_mm);
                for (axis, text) in ["X", "Y", "Z"].iter().zip(self.tools.move_to.iter_mut()) {
                    ui.horizontal(|ui| {
                        ui.label(*axis);
                        ui.add(
                            egui::TextEdit::singleline(text)
                                .desired_width(80.0)
                                .char_limit(MAX_TEXT),
                        );
                    });
                }
                if let Some(p) = placement {
                    ui.label(format!("Element at {},{},{} mm.", p[0], p[1], p[2]));
                }
            }
            ToolKind::Delete => {
                self.selection_line(ui);
                ui.colored_label(
                    RED,
                    "Commit removes the whole element (no undo in this window; the engine refuses a delete that would leave a link behind).",
                );
            }
        }
        let store = self.source.store.clone();
        let busy = self.tools.busy();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("Preview"))
                .clicked()
            {
                match self.tools.preview(&store) {
                    Ok(_) => self.tool_note = None,
                    Err(e) => {
                        self.tools.cancel();
                        self.tool_note = Some(Err(e.to_string()));
                    }
                }
            }
            let can_commit = self.tools.can_commit(&store);
            if ui
                .add_enabled(can_commit, egui::Button::new("Commit"))
                .clicked()
            {
                self.start_commit();
            }
            if ui
                .add_enabled(
                    self.tools.staged().is_some() && !busy,
                    egui::Button::new("Cancel"),
                )
                .clicked()
            {
                self.tools.cancel();
                self.tool_note = Some(Ok("Cancelled; nothing was published.".into()));
            }
        });
        if let Some(staged) = self.tools.staged() {
            let current = self.tools.can_commit(&store);
            ui.label(format!(
                "Preview: Commit runs `beamcraft` with exactly these {} arguments, one per line (not a shell command line). The engine builds the shape then; there is no geometry preview.",
                staged.args.len()
            ));
            ui.add(
                egui::Label::new(egui::RichText::new(staged.argument_list()).monospace()).wrap(),
            );
            if !current {
                ui.colored_label(RED, "The parameters changed: preview again.");
            }
        }
        if busy {
            ui.label("Running…");
        }
        if self.reload_job.is_some() {
            ui.label("Refreshing the model and definitions…");
        }
        if let Some(problem) = &self.refresh_error {
            ui.colored_label(RED, problem);
        }
        match &self.tool_note {
            Some(Ok(message)) => {
                ui.label(message);
            }
            Some(Err(message)) => {
                ui.colored_label(RED, message);
            }
            None => {}
        }
    }

    fn info(&self, ui: &mut egui::Ui) {
        ui.heading("BeamCraft");
        ui.label(format!("{:.0} FPS", self.fps));
        ui.label(&self.adapter);
        ui.label(format!("{}x MSAA", self.renderer.samples));
        ui.separator();
        match &self.loaded {
            Some(l) => {
                let pin = l.snapshot.pin();
                let (meshes, instances) = l.scene.counts();
                let revision = pin.revision().to_string();
                ui.label(format!(
                    "Revision {}… generation {}",
                    revision.get(..12).unwrap_or(&revision),
                    pin.generation()
                ));
                ui.label(format!("Elements: {}", l.scene.elements));
                ui.label(format!("Meshes: {meshes} (instances: {instances})"));
                ui.label(format!("Triangles: {}", l.scene.triangles));
                let size = [0, 1, 2].map(|a| {
                    f64::from(l.scene.max.get(a).copied().unwrap_or(0.0))
                        - f64::from(l.scene.min.get(a).copied().unwrap_or(0.0))
                });
                ui.label(format!(
                    "Bounds: {:.3} x {:.3} x {:.3} m (model units: mm)",
                    size[0] / 1000.0,
                    size[1] / 1000.0,
                    size[2] / 1000.0
                ));
            }
            None => {
                ui.label("No model loaded");
            }
        }
        ui.separator();
        ui.label("Drag: orbit | secondary drag: pan | wheel: zoom | F: fit | R: reload");
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::from_rgb(230, 80, 60), error);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32();
        self.last = now;
        if dt > 0.0 {
            self.fps = self.fps * 0.9 + 0.1 / dt;
        }
        let wants_text = ui.ctx().egui_wants_keyboard_input();
        let (fit, reload) = ui.input(|i| {
            (
                !wants_text && i.key_pressed(egui::Key::F),
                !wants_text && i.key_pressed(egui::Key::R),
            )
        });
        self.poll_commit();
        self.poll_reload();
        if reload {
            self.reload();
        }
        if fit && let Some(l) = &self.loaded {
            let (target, extent) = l.scene.framing();
            if let Err(e) = self.camera.fit(target, extent, self.aspect()) {
                self.error = Some(e.to_string());
            }
        }
        if self.text {
            egui::Panel::left("info").show(ui, |ui| self.panel(ui));
        }
        egui::CentralPanel::default().show(ui, |ui| {
            let size = ui.available_size();
            let scale = ui.ctx().pixels_per_point();
            let width = ((size.x * scale) as u32).clamp(1, 4096);
            let height = ((size.y * scale) as u32).clamp(1, 4096);
            if let Err(e) = self.resize(width, height) {
                self.error = Some(e.to_string());
            }
            let response =
                ui.add(egui::Image::new((self.texture, size)).sense(egui::Sense::drag()));
            let delta = ui.input(|i| i.pointer.delta());
            let orbit = if response.dragged_by(egui::PointerButton::Primary) {
                [delta.x * 0.008, -delta.y * 0.008]
            } else {
                [0.0; 2]
            };
            let pan = if response.dragged_by(egui::PointerButton::Secondary) {
                let factor = self.camera.distance() * 0.0008;
                [-delta.x * factor, delta.y * factor]
            } else {
                [0.0; 2]
            };
            let zoom = if response.hovered() {
                ui.input(|i| i.smooth_scroll_delta.y * 0.002)
            } else {
                0.0
            };
            let aspect = self.aspect();
            match self
                .camera
                .navigate(orbit, pan, zoom)
                .and_then(|()| self.camera.view(aspect))
            {
                Ok(view) => {
                    if let Some(l) = &self.loaded {
                        self.renderer.draw(
                            &self.state.device,
                            &self.state.queue,
                            &l.uploaded,
                            view,
                        );
                    }
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        });
        ui.ctx().request_repaint();
    }
}
