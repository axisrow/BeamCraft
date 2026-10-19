// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The eframe window: a side panel with the revision, counts, units and adapter, the viewport
//! with orbit (primary drag), pan (secondary drag), zoom (wheel), fit (F), reload (R), click
//! selection and the edit tools (V05.2), the create tools (V05.1), the properties panel and the
//! publication panel of a personal workspace (V06).

use crate::camera::{Camera, View};
use crate::command::Selected;
use crate::edit::{EditKind, Edited, Editor, Epoch, Target, ndc_of, ray_through};
use crate::props::{self, MaterialEdit, Properties, WorkspacePanel};
use crate::render::{Renderer, Uploaded};
use crate::scene::Scene;
use crate::source::Reloaded;
use crate::tools::{Choice, Created, MAX_TEXT, MemberKind, ToolError, ToolKind, Tools};
use crate::{Error, check_font, command, source};
use beamcraft_engine::ViewSnapshot;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Instant;

/// Where the window reads its model from.
#[derive(Debug, Clone)]
pub struct Source {
    /// The store the window edits and shows: the published store, or the personal workspace's
    /// directory when one is open.
    pub store: PathBuf,
    /// The `beamcraft` CLI that exports the snapshot and runs the commands.
    pub cli: PathBuf,
    /// The personal workspace directory when the window was opened on one (`--workspace`): it is
    /// then also `store`, and undo and redo act on it.
    pub workspace: Option<PathBuf>,
    /// The published store the workspace publishes to, when named (`--store` together with
    /// `--workspace`, V06): it enables the publication panel.
    pub published: Option<PathBuf>,
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
    error: Option<String>,
    text: bool,
    adapter: String,
    last: Instant,
    fps: f32,
    tools: Tools,
    job: Option<Receiver<Result<Created, ToolError>>>,
    reload_job: Option<Receiver<Reloaded>>,
    reload_queued: bool,
    /// The element id the running refresh reads the properties of, so a stale refresh never
    /// overwrites a newer selection.
    reload_selection: Option<String>,
    /// The result of the last Preview, Cancel or Commit.
    tool_note: Option<Result<String, String>>,
    /// Why the last definitions refresh failed; kept apart from `tool_note` so neither hides the other.
    refresh_error: Option<String>,
    editor: Editor,
    edit_job: Option<Receiver<Result<Edited, ToolError>>>,
    select_job: Option<SelectJob>,
    select_epoch: Epoch,
    /// The result of the last select, edit, undo or redo.
    edit_note: Option<Result<String, String>>,
    /// The selected element's full properties, as of the last pick or refresh.
    selected: Option<Properties>,
    /// The properties panel's edit state.
    props: props::Props,
    /// The running `update-element` property patch.
    props_job: Option<Receiver<Result<props::Change, ToolError>>>,
    /// The result of the last Preview, Cancel or Commit of the properties panel.
    props_note: Option<Result<String, String>>,
    /// Why the last properties refresh failed, or that the element is gone; kept apart from
    /// `props_note` and from the edit panel's note.
    props_problem: Option<String>,
    /// The publication panel as of the last refresh.
    ws_panel: Option<Result<WorkspacePanel, ToolError>>,
    /// The running `workspace-publish`.
    ws_job: Option<Receiver<Result<props::Change, ToolError>>>,
    /// The result of the last publish.
    ws_note: Option<Result<String, String>>,
    /// True while any change command (create, edit, update, publish) runs; the other panels'
    /// commit buttons wait for it, so one store never sees two change commands.
    mutating: bool,
}

/// A lookup in flight: the epoch it was asked under and its receiver. The answer carries the
/// editor's light selection and the properties panel's full report for the same element.
type SelectJob = (
    u64,
    Receiver<(
        Result<Option<Selected>, ToolError>,
        Result<Option<Properties>, ToolError>,
    )>,
);

const RED: egui::Color32 = egui::Color32::from_rgb(230, 80, 60);

/// The panel note for a successful change reply the edit panel does not stage (a property patch,
/// a publish): what happened, and the accepted revision's sequence when the reply reports one.
fn change_note(change: &props::Change) -> String {
    let what = match change.outcome.as_str() {
        "element_updated" => "The update was accepted".to_owned(),
        "element_update_replayed" => {
            "The update was already applied; nothing new was written".to_owned()
        }
        "workspace_published" => "The draft head is published".to_owned(),
        "workspace_publish_replayed" => "The draft head was already the published head".to_owned(),
        other => return format!("Outcome {other}."),
    };
    match change.sequence {
        Some(sequence) => format!("{what}; revision sequence {sequence}."),
        None => format!("{what}."),
    }
}

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

fn point_fields(ui: &mut egui::Ui, point: &mut [String; 3]) {
    ui.horizontal(|ui| {
        for (axis, text) in ["X", "Y", "Z"].iter().zip(point.iter_mut()) {
            ui.label(*axis);
            ui.add(
                egui::TextEdit::singleline(text)
                    .desired_width(64.0)
                    .char_limit(MAX_TEXT),
            );
        }
    });
}

/// The twelve edges of a box as pairs of corner indices (bit 0 = x, bit 1 = y, bit 2 = z).
const BOX_EDGES: [(usize, usize); 12] = [
    (0, 1),
    (2, 3),
    (4, 5),
    (6, 7),
    (0, 2),
    (1, 3),
    (4, 6),
    (5, 7),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];

/// Draws the selection as the projected box of the engine's world bounds of the element.
fn draw_selection(
    painter: &egui::Painter,
    rect: egui::Rect,
    view: &View,
    loaded: &Loaded,
    id: &str,
) {
    let Ok(id) = beamcraft_engine::ElementId::parse(id) else {
        return;
    };
    let Ok(bounds) = loaded.snapshot.element_bounds(&id) else {
        return;
    };
    let origin = loaded.scene.origin_mm;
    let screen: Vec<Option<egui::Pos2>> = (0..8usize)
        .map(|corner| {
            let pick = |axis: usize| {
                let on = corner >> axis & 1 == 1;
                let edge = if on {
                    bounds.max.get(axis)
                } else {
                    bounds.min.get(axis)
                };
                edge.copied().unwrap_or(0.0) - origin.get(axis).copied().unwrap_or(0.0)
            };
            let [x, y, z] = view.project([pick(0) as f32, pick(1) as f32, pick(2) as f32]);
            ((0.0..1.0).contains(&z) && x.is_finite() && y.is_finite()).then(|| {
                egui::pos2(
                    rect.min.x + (x * 0.5 + 0.5) * rect.width(),
                    rect.min.y + (0.5 - y * 0.5) * rect.height(),
                )
            })
        })
        .collect();
    let stroke = egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 190, 40));
    for (a, b) in BOX_EDGES {
        if let (Some(Some(p)), Some(Some(q))) = (screen.get(a), screen.get(b)) {
            painter.line_segment([*p, *q], stroke);
        }
    }
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
    // CWD (an extracted package root), the source tree, then next to the executable: the
    // win/linux package puts assets beside it, the macOS .app bundle under its Resources
    // directory (Q02 packaging).
    let mut candidates = vec![
        Path::new("assets/fonts/inter/Inter-Regular.ttf").to_path_buf(),
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts/inter/Inter-Regular.ttf"),
    ];
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("assets/fonts/inter/Inter-Regular.ttf"));
        candidates.push(dir.join("../Resources/assets/fonts/inter/Inter-Regular.ttf"));
    }
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
            error: None,
            text,
            adapter,
            last: Instant::now(),
            fps: 0.0,
            tools: Tools::default(),
            job: None,
            reload_job: None,
            reload_queued: false,
            reload_selection: None,
            tool_note: None,
            refresh_error: None,
            editor: Editor::default(),
            edit_job: None,
            select_job: None,
            select_epoch: Epoch::default(),
            edit_note: None,
            selected: None,
            props: props::Props::default(),
            props_job: None,
            props_note: None,
            props_problem: None,
            ws_panel: None,
            ws_job: None,
            ws_note: None,
            mutating: false,
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
        let selected = self.editor.selected().map(|s| s.id.clone());
        let for_worker = selected.clone();
        let workspace = self
            .source
            .published
            .clone()
            .zip(self.source.workspace.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("beamcraft-reload".into())
            .spawn(move || {
                let _ = tx.send(source::reload(
                    &cli,
                    &store,
                    for_worker.as_deref(),
                    workspace.as_ref().map(|(p, d)| (p.as_path(), d.as_path())),
                ));
            });
        match spawned {
            Ok(_) => {
                self.reload_job = Some(rx);
                self.reload_selection = selected;
            }
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
        self.ws_panel = reloaded.workspace;
        // A refresh that started for another selection never overwrites a newer one.
        let mine = self.reload_selection.take();
        let current = self.editor.selected().map(|s| s.id.as_str());
        if mine.as_deref() == current {
            match reloaded.properties {
                Some(Ok(Some(properties))) => {
                    self.selected = Some(properties);
                    self.props_problem = None;
                }
                Some(Ok(None)) => {
                    if self.editor.selected().is_some() {
                        self.selected = None;
                        self.props = props::Props::default();
                        self.props_problem =
                            Some("the selected element is no longer in the store".into());
                    }
                }
                Some(Err(e)) => {
                    self.props_problem = Some(format!("properties: {e}"));
                }
                None => {}
            }
        }
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
            ui.separator();
            self.edit_panel(ui);
            ui.separator();
            self.properties_panel(ui);
            if self.source.published.is_some() {
                ui.separator();
                self.workspace_panel(ui);
            }
        });
    }

    fn target(&self) -> Target<'_> {
        Target {
            store: &self.source.store,
            workspace: self.source.workspace.as_deref(),
        }
    }

    /// Starts a click selection on a worker: the engine's `pick` along the ray, then the details
    /// of the element it hit — the editor's light report and the properties panel's full one.
    fn start_select(&mut self, ray: crate::edit::Ray) {
        if self.select_job.is_some() {
            return;
        }
        let (cli, store) = (self.source.cli.clone(), self.source.store.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        self.select_job = Some((self.select_epoch.advance(), rx));
        let spawned = std::thread::Builder::new()
            .name("beamcraft-pick".into())
            .spawn(move || {
                let picked = command::select_at(&cli, &store, &ray);
                let properties = match &picked {
                    Ok(Some(found)) => command::properties(&cli, &store, &found.id),
                    _ => Ok(None),
                };
                let _ = tx.send((picked, properties));
            });
        if spawned.is_err() {
            self.select_job = None;
            self.edit_note = Some(Err("the pick thread could not be started".into()));
        }
    }

    /// Drops any lookup in flight: its result is no longer wanted (Esc, an accepted edit).
    fn invalidate_select(&mut self) {
        self.select_epoch.advance();
        self.select_job = None;
    }

    fn poll_select(&mut self) {
        let Some((asked, rx)) = &self.select_job else {
            return;
        };
        let asked = *asked;
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => (
                Err(ToolError {
                    code: "CLI_FAILED".into(),
                    message: "the pick thread ended without a result".into(),
                }),
                Ok(None),
            ),
        };
        self.select_job = None;
        if !self.select_epoch.is_current(asked) {
            return;
        }
        let (picked, properties) = result;
        match picked {
            Ok(None) => {
                self.editor.deselect();
                self.selected = None;
                self.edit_note = None;
            }
            Ok(Some(Selected { id, info })) => {
                // The properties report belongs to the element the editor now holds; a selection
                // that changed element starts the property patch from an empty form.
                let changed = self.editor.selected().map(|s| s.id != id).unwrap_or(true);
                match info {
                    Ok(info) => self.editor.select(info),
                    Err(e) => {
                        self.editor.select_id(id);
                        self.edit_note = Some(Err(format!("details: {e}")));
                    }
                }
                if changed {
                    self.props = props::Props::default();
                    self.props_note = None;
                }
                match properties {
                    Ok(Some(properties)) => {
                        self.selected = Some(properties);
                        self.props_problem = None;
                    }
                    Ok(None) => {
                        self.selected = None;
                        self.props_problem = Some(
                            "the properties of the selected element are not in the report".into(),
                        );
                    }
                    Err(e) => {
                        self.props_problem = Some(format!("properties: {e}"));
                    }
                }
            }
            Err(e) => self.edit_note = Some(Err(e.to_string())),
        }
    }

    fn start_edit(&mut self, direct: Option<EditKind>) {
        let target = Target {
            store: &self.source.store,
            workspace: self.source.workspace.as_deref(),
        };
        let edit = match direct {
            Some(kind) => match self.editor.begin_direct(kind, target) {
                Ok(edit) => edit,
                Err(e) => {
                    self.edit_note = Some(Err(e.to_string()));
                    return;
                }
            },
            None => self.editor.begin_commit(target),
        };
        let Some(edit) = edit else { return };
        let cli = self.source.cli.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.edit_job = Some(rx);
        self.edit_note = None;
        self.mutating = true;
        let spawned = std::thread::Builder::new()
            .name("beamcraft-edit".into())
            .spawn(move || {
                let _ = tx.send(command::commit_edit(&cli, &edit));
            });
        if spawned.is_err() {
            self.edit_job = None;
            self.editor.finish();
            self.mutating = false;
            self.edit_note = Some(Err("the edit thread could not be started".into()));
        }
    }

    fn poll_edit(&mut self) {
        let Some(rx) = &self.edit_job else { return };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(ToolError {
                code: "CLI_FAILED".into(),
                message: "the edit thread ended without a result".into(),
            }),
        };
        self.edit_job = None;
        self.editor.finish();
        self.mutating = false;
        match result {
            Ok(done) => {
                // A lookup that started before this edit describes the element as it was.
                self.invalidate_select();
                self.reload();
                let subject = match done.kind {
                    EditKind::Undo | EditKind::Redo => "the workspace".to_owned(),
                    _ => done.element_id.clone().unwrap_or_default(),
                };
                self.edit_note = Some(Ok(format!(
                    "{} {} (accepted revision {}, sequence {}).",
                    done.kind.done(),
                    subject,
                    done.manifest_digest
                        .get(..12)
                        .unwrap_or(&done.manifest_digest),
                    done.sequence
                )));
                match done.kind {
                    EditKind::Delete => {
                        self.editor.deselect();
                        self.selected = None;
                        self.props = props::Props::default();
                    }
                    // The element may have moved, or the undo may have removed it: ask again.
                    _ => self.refresh_selection(),
                }
            }
            Err(e) => {
                if e.code == "OUTCOME_UNKNOWN" {
                    self.invalidate_select();
                    self.reload();
                    self.refresh_selection();
                }
                self.edit_note = Some(Err(e.to_string()));
            }
        }
    }

    /// Re-reads the selected element from the accepted revision (it may have moved or gone).
    fn refresh_selection(&mut self) {
        let Some(id) = self.editor.selected().map(|s| s.id.clone()) else {
            return;
        };
        let (cli, store) = (self.source.cli.clone(), self.source.store.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        self.select_job = Some((self.select_epoch.advance(), rx));
        let spawned = std::thread::Builder::new()
            .name("beamcraft-info".into())
            .spawn(move || {
                // An element the revision no longer holds clears the selection; any other
                // failure keeps it selected without details, so the move form is emptied
                // rather than left with the position from before the edit.
                let reply = match command::element_info(&cli, &store, &id) {
                    Err(e) if e.code == "NOT_FOUND" => None,
                    info => Some(Selected {
                        id: id.clone(),
                        info,
                    }),
                };
                let properties = command::properties(&cli, &store, &id);
                let _ = tx.send((Ok(reply), properties));
            });
        if spawned.is_err() {
            self.select_job = None;
        }
    }

    fn edit_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Edit");
        ui.label("Click an element in the view to select it. Esc clears the selection.");
        match self.editor.selected().cloned() {
            Some(info) => {
                ui.label(format!(
                    "Selected: {} {} ({})",
                    info.kind, info.name, info.id
                ));
            }
            None => {
                ui.label("Nothing selected.");
            }
        }
        let selected = self.editor.selected().is_some();
        let mut action = self.editor.action();
        ui.add_enabled_ui(selected, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut action, EditKind::Move, "Move");
                ui.selectable_value(&mut action, EditKind::Copy, "Copy");
                ui.selectable_value(&mut action, EditKind::Delete, "Delete");
            });
            match action {
                EditKind::Move => {
                    ui.label("New position in mm (absolute):");
                    point_fields(ui, &mut self.editor.move_to);
                }
                EditKind::Copy => {
                    ui.label("Offset of the copy in mm:");
                    point_fields(ui, &mut self.editor.copy_offset);
                }
                _ => {
                    ui.label("Deletes the element (refused while another element links to it).");
                }
            }
        });
        // Another action needs its own preview.
        self.editor.set_action(action);
        let busy = self.editor.busy();
        let target = self.target();
        let can_commit = self.editor.can_commit(target) && !self.mutating;
        let staged = self.editor.staged().is_some();
        let (mut preview, mut commit, mut cancel) = (false, false, false);
        ui.horizontal(|ui| {
            preview = ui
                .add_enabled(selected && !busy, egui::Button::new("Preview"))
                .clicked();
            commit = ui
                .add_enabled(can_commit, egui::Button::new("Commit"))
                .clicked();
            cancel = ui
                .add_enabled(staged && !busy, egui::Button::new("Cancel"))
                .clicked();
        });
        if preview {
            let action = self.editor.action();
            let target = Target {
                store: &self.source.store,
                workspace: self.source.workspace.as_deref(),
            };
            match self.editor.preview(action, target) {
                Ok(_) => self.edit_note = None,
                Err(e) => {
                    self.editor.cancel();
                    self.edit_note = Some(Err(e.to_string()));
                }
            }
        }
        if commit {
            self.start_edit(None);
        }
        if cancel {
            self.editor.cancel();
            self.edit_note = Some(Ok("Cancelled; nothing was changed.".into()));
        }
        if let Some(edit) = self.editor.staged() {
            ui.label(format!(
                "Preview: Commit runs `beamcraft` with exactly these {} arguments, one per line (not a shell command line).",
                edit.args.len()
            ));
            ui.add(egui::Label::new(egui::RichText::new(edit.argument_list()).monospace()).wrap());
            if !self.editor.can_commit(self.target()) {
                ui.colored_label(RED, "The parameters changed: preview again.");
            }
        }
        let has_workspace = self.source.workspace.is_some();
        let (mut undo, mut redo) = (false, false);
        ui.horizontal(|ui| {
            undo = ui
                .add_enabled(
                    has_workspace && !busy && !self.mutating,
                    egui::Button::new("Undo"),
                )
                .clicked();
            redo = ui
                .add_enabled(
                    has_workspace && !busy && !self.mutating,
                    egui::Button::new("Redo"),
                )
                .clicked();
        });
        if undo {
            self.start_edit(Some(EditKind::Undo));
        }
        if redo {
            self.start_edit(Some(EditKind::Redo));
        }
        if !has_workspace {
            ui.label(
                "Undo and redo need a personal workspace: open the window with --workspace DIR.",
            );
        }
        if busy {
            ui.label("Running…");
        }
        if self.select_job.is_some() {
            ui.label("Looking up the element…");
        }
        match &self.edit_note {
            Some(Ok(message)) => {
                ui.label(message);
            }
            Some(Err(message)) => {
                ui.colored_label(RED, message);
            }
            None => {}
        }
    }

    fn start_commit(&mut self) {
        let Some(prepared) = self.tools.begin_commit(&self.source.store) else {
            return;
        };
        let cli = self.source.cli.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.job = Some(rx);
        self.tool_note = None;
        self.mutating = true;
        let spawned = std::thread::Builder::new()
            .name("beamcraft-command".into())
            .spawn(move || {
                let _ = tx.send(command::commit(&cli, &prepared));
            });
        if spawned.is_err() {
            self.job = None;
            self.tools.finish();
            self.mutating = false;
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
        self.mutating = false;
        match result {
            Ok(created) => {
                self.reload();
                let what = match created.kind {
                    ToolKind::Plate => "plate",
                    ToolKind::Member => "member",
                };
                let volume = created
                    .volume_mm3
                    .map_or(String::new(), |v| format!(", volume {:.3} m3", v / 1e9));
                self.tool_note = Some(Ok(format!(
                    "Created {what} {} ({}{}). Accepted revision {} (sequence {}).",
                    created.element_id,
                    if created.name.is_empty() {
                        "unnamed"
                    } else {
                        &created.name
                    },
                    volume,
                    created
                        .manifest_digest
                        .get(..12)
                        .unwrap_or(&created.manifest_digest),
                    created.sequence
                )));
            }
            Err(e) => {
                if e.code == "OUTCOME_UNKNOWN" {
                    self.reload();
                }
                self.tool_note = Some(Err(e.to_string()));
            }
        }
    }

    fn tools_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Create");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tools.active, ToolKind::Plate, "Plate");
            ui.selectable_value(&mut self.tools.active, ToolKind::Member, "Member");
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

    /// Runs the properties panel's staged `update-element` property patch.
    fn start_update(&mut self) {
        let store = self.source.store.clone();
        let Some(id) = self.selected.as_ref().map(|p| p.element_id.clone()) else {
            return;
        };
        let Some(args) = self.props.begin_commit(&store, &id) else {
            return;
        };
        let cli = self.source.cli.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.props_job = Some(rx);
        self.props_note = None;
        self.mutating = true;
        let spawned = std::thread::Builder::new()
            .name("beamcraft-update".into())
            .spawn(move || {
                let _ = tx.send(command::run_change(&cli, &args));
            });
        if spawned.is_err() {
            self.props_job = None;
            self.props.finish();
            self.mutating = false;
            self.props_note = Some(Err("the command thread could not be started".into()));
        }
    }

    fn poll_update(&mut self) {
        let Some(rx) = &self.props_job else { return };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(ToolError::local(
                "CLI_FAILED",
                "the command thread ended without a result",
            )),
        };
        self.props_job = None;
        self.props.finish();
        self.mutating = false;
        match result {
            Ok(change) => {
                self.invalidate_select();
                self.reload();
                self.refresh_selection();
                self.props_note = Some(Ok(change_note(&change)));
            }
            Err(e) => {
                if e.code == "OUTCOME_UNKNOWN" {
                    self.invalidate_select();
                    self.reload();
                    self.refresh_selection();
                }
                self.props_note = Some(Err(e.to_string()));
            }
        }
    }

    /// Runs `workspace-publish`: the draft head becomes the next published revision,
    /// fast-forward only. It takes no form, so there is nothing to preview.
    fn start_workspace(&mut self) {
        let (Some(published), Some(dir)) =
            (self.source.published.clone(), self.source.workspace.clone())
        else {
            return;
        };
        if self.ws_job.is_some() || self.mutating {
            return;
        }
        let cli = self.source.cli.clone();
        let args = props::workspace_args("workspace-publish", &published, &dir);
        let (tx, rx) = std::sync::mpsc::channel();
        self.ws_job = Some(rx);
        self.ws_note = None;
        self.mutating = true;
        let spawned = std::thread::Builder::new()
            .name("beamcraft-publish".into())
            .spawn(move || {
                let _ = tx.send(command::run_change(&cli, &args));
            });
        if spawned.is_err() {
            self.ws_job = None;
            self.mutating = false;
            self.ws_note = Some(Err("the command thread could not be started".into()));
        }
    }

    fn poll_workspace(&mut self) {
        let Some(rx) = &self.ws_job else { return };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(ToolError::local(
                "CLI_FAILED",
                "the command thread ended without a result",
            )),
        };
        self.ws_job = None;
        self.mutating = false;
        match result {
            Ok(change) => {
                self.reload();
                self.ws_note = Some(Ok(change_note(&change)));
            }
            Err(e) => {
                if e.code == "OUTCOME_UNKNOWN" {
                    self.reload();
                }
                self.ws_note = Some(Err(e.to_string()));
            }
        }
    }

    /// The selected element's full properties, with the typed property patch beneath them. The
    /// position itself is the edit panel's Move action; here it is shown read-only.
    fn properties_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Properties");
        if let Some(problem) = &self.props_problem {
            ui.colored_label(RED, problem);
        }
        let Some(selected) = self.selected.clone() else {
            if self.editor.selected().is_some() && self.select_job.is_none() {
                ui.label("The properties of the selection are not loaded.");
            } else {
                ui.label("Select an element in the view to see its properties.");
            }
            match &self.props_note {
                Some(Ok(message)) => {
                    ui.label(message);
                }
                Some(Err(message)) => {
                    ui.colored_label(RED, message);
                }
                None => {}
            }
            return;
        };
        ui.monospace(format!(
            "{} ({})",
            if selected.name.is_empty() {
                "unnamed"
            } else {
                &selected.name
            },
            selected.kind
        ));
        let dimensions = selected
            .dimensions
            .iter()
            .map(|(key, value)| format!("{key} {value} mm"))
            .collect::<Vec<_>>()
            .join(", ");
        if !dimensions.is_empty() {
            ui.label(dimensions);
        }
        if let Some(profile) = &selected.profile {
            ui.label(format!("Profile {}", profile.label()));
        }
        ui.label(match &selected.material {
            Some(material) => format!("Material {}", material.label()),
            None => "Material none".to_owned(),
        });
        ui.label(match selected.volume_mm3 {
            Some(volume) => {
                format!("Volume {:.3} m3 ({})", volume / 1e9, selected.volume_status)
            }
            None => format!("Volume none ({})", selected.volume_status),
        });
        ui.label(match selected.mass_kg {
            Some(mass) => format!("Mass {mass:.3} kg ({})", selected.mass_status),
            None => format!("Mass none ({})", selected.mass_status),
        });
        ui.label(format!(
            "Position {:.3}, {:.3}, {:.3} mm (move it with the Move action above)",
            selected.translation_mm[0], selected.translation_mm[1], selected.translation_mm[2]
        ));
        // The typed patch; every field left as it came changes nothing.
        ui.separator();
        ui.label("Edit (empty fields change nothing):");
        let materials = self.tools.definitions.materials.clone();
        let store = self.source.store.clone();
        let busy = self.props.busy() || self.mutating;
        field(ui, "Name", &mut self.props.form.name, 140.0);
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("edit-material")
                .width(200.0)
                .selected_text(match &self.props.form.material {
                    MaterialEdit::Keep => "keep current".to_owned(),
                    MaterialEdit::Clear => "none".to_owned(),
                    MaterialEdit::Set(digest) => materials
                        .iter()
                        .find(|choice| &choice.digest == digest)
                        .map_or_else(
                            || format!("…{}", digest.get(..8).unwrap_or(digest)),
                            |choice| choice.label.clone(),
                        ),
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.props.form.material,
                        MaterialEdit::Keep,
                        "keep current",
                    );
                    ui.selectable_value(
                        &mut self.props.form.material,
                        MaterialEdit::Clear,
                        "none (remove the material)",
                    );
                    for choice in &materials {
                        ui.selectable_value(
                            &mut self.props.form.material,
                            MaterialEdit::Set(choice.digest.clone()),
                            &choice.label,
                        );
                    }
                });
            ui.label("Material");
        });
        if selected.kind == "plate" {
            field(ui, "Thickness", &mut self.props.form.thickness, 80.0);
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("Preview"))
                .clicked()
            {
                match self.props.preview(&store, &selected.element_id) {
                    Ok(()) => self.props_note = None,
                    Err(e) => {
                        self.props.cancel();
                        self.props_note = Some(Err(e.to_string()));
                    }
                }
            }
            let can_commit = self.props.can_commit(&store, &selected.element_id) && !self.mutating;
            if ui
                .add_enabled(can_commit, egui::Button::new("Commit"))
                .clicked()
            {
                self.start_update();
            }
            if ui
                .add_enabled(
                    self.props.staged().is_some() && !busy,
                    egui::Button::new("Cancel"),
                )
                .clicked()
            {
                self.props.cancel();
                self.props_note = Some(Ok("Cancelled; nothing was published.".into()));
            }
        });
        if let Some(staged) = self.props.staged() {
            let current = self.props.can_commit(&store, &selected.element_id);
            ui.label(format!(
                "Preview: Commit runs `beamcraft` with exactly these {} arguments, one per line (not a shell command line).",
                staged.len()
            ));
            let list = staged
                .iter()
                .map(|argument| crate::json_string(argument))
                .collect::<Vec<_>>()
                .join("\n");
            ui.add(egui::Label::new(egui::RichText::new(list).monospace()).wrap());
            if !current {
                ui.colored_label(RED, "The parameters changed: preview again.");
            }
        }
        if self.props_job.is_some() {
            ui.label("Running…");
        }
        match &self.props_note {
            Some(Ok(message)) => {
                ui.label(message);
            }
            Some(Err(message)) => {
                ui.colored_label(RED, message);
            }
            None => {}
        }
    }

    /// The personal workspace's publication panel: where the draft stands against the named
    /// published store, and the fast-forward-only publish. Undo and redo are the edit panel's.
    fn workspace_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Workspace");
        match &self.ws_panel {
            None => {
                ui.label("Reading the workspace status…");
            }
            Some(Err(e)) => {
                ui.colored_label(RED, e.to_string());
            }
            Some(Ok(panel)) => {
                let panel = panel.clone();
                let (draft, draft_sequence) = &panel.state.draft_head;
                let (published, published_sequence) = &panel.state.published_head;
                ui.label(format!(
                    "Draft head {}… (sequence {draft_sequence})",
                    draft.get(..12).unwrap_or(draft)
                ));
                ui.label(format!(
                    "Published head {}… (sequence {published_sequence})",
                    published.get(..12).unwrap_or(published)
                ));
                ui.label(format!(
                    "Unsent revisions: {}",
                    panel.state.unsent_revisions
                ));
                if panel.state.base_is_current {
                    ui.label("The published head is still the base.");
                } else {
                    ui.colored_label(
                        RED,
                        "The published head moved; publish is refused (BASE_REVISION_MISMATCH, fast-forward only).",
                    );
                }
                if ui
                    .add_enabled(
                        !self.mutating && self.ws_job.is_none() && panel.state.unsent_revisions > 0,
                        egui::Button::new("Publish"),
                    )
                    .clicked()
                {
                    self.start_workspace();
                }
                if self.ws_job.is_some() {
                    ui.label("Running…");
                }
                match &self.ws_note {
                    Some(Ok(message)) => {
                        ui.label(message);
                    }
                    Some(Err(message)) => {
                        ui.colored_label(RED, message);
                    }
                    None => {}
                }
            }
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
        self.poll_edit();
        self.poll_update();
        self.poll_workspace();
        self.poll_select();
        self.poll_reload();
        if !wants_text && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.invalidate_select();
            self.editor.deselect();
            self.selected = None;
            self.props = props::Props::default();
        }
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
                ui.add(egui::Image::new((self.texture, size)).sense(egui::Sense::click_and_drag()));
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
            let mut picked = None;
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
                    if let Some(l) = &self.loaded {
                        if let Some(selected) = self.editor.selected() {
                            let painter = ui.painter_at(response.rect);
                            draw_selection(&painter, response.rect, &view, l, &selected.id);
                        }
                        // A click (not a drag) picks along the ray through the pixel.
                        if response.clicked_by(egui::PointerButton::Primary)
                            && let Some(pos) = response.interact_pointer_pos()
                            && let Some(ndc) = ndc_of(
                                [pos.x, pos.y],
                                [response.rect.min.x, response.rect.min.y],
                                [response.rect.width(), response.rect.height()],
                            )
                            && let Some(ray) = ray_through(&view, ndc, l.scene.origin_mm)
                        {
                            picked = Some(ray);
                        }
                    }
                }
                Err(e) => self.error = Some(e.to_string()),
            }
            if let Some(ray) = picked {
                self.start_select(ray);
            }
        });
        ui.ctx().request_repaint();
    }
}
