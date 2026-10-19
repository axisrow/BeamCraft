// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The eframe window: a side panel with the revision, counts, units and adapter, and the viewport
//! with orbit (primary drag), pan (Shift + primary drag, or secondary drag), zoom (wheel or
//! trackpad scroll, pinch, `+` and `-`), fit (F) and reload (R).

use crate::camera::{Camera, View};
use crate::command::Selected;
use crate::edit::{EditKind, Edited, Editor, Epoch, Target, ndc_of, ray_through};
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
    /// The published store.
    pub store: PathBuf,
    /// The `beamcraft` CLI that exports the snapshot.
    pub cli: PathBuf,
    /// The personal workspace directory when the window was opened on one (`--workspace`): it is
    /// then also `store`, and undo and redo act on it.
    pub workspace: Option<PathBuf>,
}

struct Loaded {
    snapshot: ViewSnapshot,
    scene: Scene,
    uploaded: Uploaded,
}

/// The window state.
pub struct App {
    state: eframe::egui_wgpu::RenderState,
    /// The egui context, cloned into worker threads so a finished job can wake the UI: polling
    /// alone would idle past a result once the window redraws only on demand.
    ctx: egui::Context,
    source: Source,
    renderer: Renderer,
    texture: egui::TextureId,
    camera: Camera,
    loaded: Option<Loaded>,
    /// Whether the 3D image must be redrawn at the next opportunity: the scene, the camera, the
    /// target size or the renderer changed. Unrelated UI frames reuse the offscreen texture.
    viewport_dirty: bool,
    error: Option<String>,
    text: bool,
    adapter: String,
    last_draw: Instant,
    fps: f32,
    tools: Tools,
    job: Option<Receiver<Result<Created, ToolError>>>,
    reload_job: Option<Receiver<Reloaded>>,
    reload_queued: bool,
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
    /// Flips the scroll zoom direction (macOS natural scrolling inverts it); pinch and keys keep theirs.
    invert_zoom: bool,
}

/// One frame of viewport navigation in `Camera::navigate` argument space: orbit radians, a
/// screen-relative pan in millimetres and logarithmic zoom. Shift turns the primary drag into a
/// pan, so a trackpad pans without the secondary button. The scroll zoom sign follows
/// `invert_zoom`; pinch zoom and the `+`/`-` keys keep their direction. Scroll and pinch only
/// count while the viewport is hovered. A hostile pinch factor is dropped, not propagated, so
/// what comes back is always finite and inside what `navigate` accepts.
#[allow(clippy::too_many_arguments)] // one flat mapping from one frame of input to navigate's arguments
fn navigation(
    pointer_delta: egui::Vec2,
    primary: bool,
    secondary: bool,
    shift: bool,
    distance: f32,
    hovered: bool,
    scroll_y: f32,
    pinch: f32,
    zoom_in: bool,
    zoom_out: bool,
    invert_zoom: bool,
) -> ([f32; 2], [f32; 2], f32) {
    let sign = if invert_zoom { -1.0 } else { 1.0 };
    let factor = distance * 0.0008;
    let pan = [-pointer_delta.x * factor, pointer_delta.y * factor];
    let (orbit, pan) = if (primary && shift) || secondary {
        ([0.0; 2], pan)
    } else if primary {
        (
            [pointer_delta.x * 0.008, -pointer_delta.y * 0.008],
            [0.0; 2],
        )
    } else {
        ([0.0; 2], [0.0; 2])
    };
    let scroll = if hovered {
        scroll_y * 0.002 * sign
    } else {
        0.0
    };
    // A pinch arrives as a multiplicative factor; the camera takes logarithmic zoom.
    let pinch = if hovered && pinch.is_finite() && pinch > 0.0 {
        pinch.ln()
    } else {
        0.0
    };
    const KEY_STEP: f32 = 0.25;
    let keys = KEY_STEP * (i32::from(zoom_in) - i32::from(zoom_out)) as f32;
    (orbit, pan, scroll + pinch + keys)
}

/// A lookup in flight: the epoch it was asked under and its receiver.
type SelectJob = (u64, Receiver<Result<Option<Selected>, ToolError>>);

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
            ctx: cc.egui_ctx.clone(),
            source,
            renderer,
            texture,
            camera: Camera::new([0.0; 3], 1000.0, 1.6)?,
            loaded: None,
            viewport_dirty: true,
            error: None,
            text,
            adapter,
            last_draw: Instant::now(),
            fps: 0.0,
            tools: Tools::default(),
            job: None,
            reload_job: None,
            reload_queued: false,
            tool_note: None,
            refresh_error: None,
            editor: Editor::default(),
            edit_job: None,
            select_job: None,
            select_epoch: Epoch::default(),
            edit_note: None,
            invert_zoom: false,
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
        let ctx = self.ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("beamcraft-reload".into())
            .spawn(move || {
                let _ = tx.send(source::reload(&cli, &store));
                // The result is only consumed by a UI frame; wake the window for it.
                ctx.request_repaint();
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
                        self.loaded = Some(loaded);
                        self.error = None;
                        self.viewport_dirty = true;
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
        self.viewport_dirty = true;
        Ok(())
    }

    fn panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            self.info(ui);
            ui.separator();
            self.tools_panel(ui);
            ui.separator();
            self.edit_panel(ui);
        });
    }

    fn target(&self) -> Target<'_> {
        Target {
            store: &self.source.store,
            workspace: self.source.workspace.as_deref(),
        }
    }

    /// Starts a click selection on a worker: the engine's `pick` along the ray, then the details
    /// of the element it hit.
    fn start_select(&mut self, ray: crate::edit::Ray) {
        if self.select_job.is_some() {
            return;
        }
        let (cli, store) = (self.source.cli.clone(), self.source.store.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = self.ctx.clone();
        self.select_job = Some((self.select_epoch.advance(), rx));
        let spawned = std::thread::Builder::new()
            .name("beamcraft-pick".into())
            .spawn(move || {
                let _ = tx.send(command::select_at(&cli, &store, &ray));
                ctx.request_repaint();
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
            Err(TryRecvError::Disconnected) => Err(ToolError {
                code: "CLI_FAILED".into(),
                message: "the pick thread ended without a result".into(),
            }),
        };
        self.select_job = None;
        if !self.select_epoch.is_current(asked) {
            return;
        }
        match result {
            Ok(None) => {
                self.editor.deselect();
                self.edit_note = None;
            }
            Ok(Some(Selected { id, info })) => match info {
                Ok(info) => {
                    self.editor.select(info);
                    self.edit_note = None;
                }
                Err(e) => {
                    self.editor.select_id(id);
                    self.edit_note = Some(Err(format!("details: {e}")));
                }
            },
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
        let ctx = self.ctx.clone();
        self.edit_job = Some(rx);
        self.edit_note = None;
        let spawned = std::thread::Builder::new()
            .name("beamcraft-edit".into())
            .spawn(move || {
                let _ = tx.send(command::commit_edit(&cli, &edit));
                ctx.request_repaint();
            });
        if spawned.is_err() {
            self.edit_job = None;
            self.editor.finish();
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
                    EditKind::Delete => self.editor.deselect(),
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
        let ctx = self.ctx.clone();
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
                let _ = tx.send(Ok(reply));
                ctx.request_repaint();
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
        let can_commit = self.editor.can_commit(target);
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
                .add_enabled(has_workspace && !busy, egui::Button::new("Undo"))
                .clicked();
            redo = ui
                .add_enabled(has_workspace && !busy, egui::Button::new("Redo"))
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
        let ctx = self.ctx.clone();
        self.job = Some(rx);
        self.tool_note = None;
        let spawned = std::thread::Builder::new()
            .name("beamcraft-command".into())
            .spawn(move || {
                let _ = tx.send(command::commit(&cli, &prepared));
                ctx.request_repaint();
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

    fn info(&mut self, ui: &mut egui::Ui) {
        ui.heading("BeamCraft");
        ui.label(format!("{:.0} viewport redraws/s", self.fps));
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
        ui.label("Drag: orbit | Shift+drag or secondary drag: pan | wheel or pinch: zoom | +/-: zoom | F: fit | R: reload");
        ui.checkbox(
            &mut self.invert_zoom,
            "Invert zoom (trackpad natural scrolling)",
        );
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::from_rgb(230, 80, 60), error);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let wants_text = ui.ctx().egui_wants_keyboard_input();
        let (fit, reload, zoom_in, zoom_out) = ui.input(|i| {
            (
                !wants_text && i.key_pressed(egui::Key::F),
                !wants_text && i.key_pressed(egui::Key::R),
                !wants_text && (i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals)),
                !wants_text && i.key_pressed(egui::Key::Minus),
            )
        });
        self.poll_commit();
        self.poll_edit();
        self.poll_select();
        self.poll_reload();
        if !wants_text && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.invalidate_select();
            self.editor.deselect();
        }
        if reload {
            self.reload();
        }
        if fit && let Some(l) = &self.loaded {
            let (target, extent) = l.scene.framing();
            if let Err(e) = self.camera.fit(target, extent, self.aspect()) {
                self.error = Some(e.to_string());
            }
            self.viewport_dirty = true;
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
            let hovered = response.hovered();
            let (delta, shift, scroll_y, pinch) = ui.input(|i| {
                (
                    i.pointer.delta(),
                    i.modifiers.shift,
                    if hovered {
                        i.smooth_scroll_delta.y
                    } else {
                        0.0
                    },
                    if hovered { i.zoom_delta() } else { 1.0 },
                )
            });
            let (orbit, pan, zoom) = navigation(
                delta,
                response.dragged_by(egui::PointerButton::Primary),
                response.dragged_by(egui::PointerButton::Secondary),
                shift,
                self.camera.distance(),
                hovered,
                scroll_y,
                pinch,
                zoom_in,
                zoom_out,
                self.invert_zoom,
            );
            let moving = orbit != [0.0; 2] || pan != [0.0; 2] || zoom != 0.0;
            let aspect = self.aspect();
            let mut picked = None;
            match self
                .camera
                .navigate(orbit, pan, zoom)
                .and_then(|()| self.camera.view(aspect))
            {
                Ok(view) => {
                    if moving {
                        self.viewport_dirty = true;
                    }
                    if self.viewport_dirty
                        && let Some(l) = &self.loaded
                    {
                        self.renderer.draw(
                            &self.state.device,
                            &self.state.queue,
                            &l.uploaded,
                            view,
                        );
                        self.viewport_dirty = false;
                        let now = Instant::now();
                        let dt = now.duration_since(self.last_draw).as_secs_f32();
                        self.last_draw = now;
                        if dt > 0.0 && dt < 5.0 {
                            self.fps = self.fps * 0.9 + 0.1 / dt;
                        }
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
        // No unconditional repaint request: the window idles until input, a finished background
        // job (`ctx.request_repaint` from the worker) or an egui animation asks for a frame, and
        // an unrelated UI frame reuses the last offscreen image.
    }
}

#[cfg(test)]
mod tests {
    use super::navigation;
    use crate::camera::Camera;

    const PLAIN: (egui::Vec2, bool, bool, bool) = (egui::Vec2::ZERO, false, false, false);
    const KEYS_NONE: (bool, bool) = (false, false);

    /// The zoom argument for key-only input at the given toggle state.
    fn key_zoom(zoom_in: bool, zoom_out: bool, invert: bool) -> f32 {
        navigation(
            PLAIN.0, PLAIN.1, PLAIN.2, PLAIN.3, 1000.0, false, 0.0, 1.0, zoom_in, zoom_out, invert,
        )
        .2
    }

    #[test]
    fn shift_turns_the_primary_drag_into_a_pan() {
        let delta = egui::Vec2::new(10.0, -20.0);
        let (orbit, pan, _) = navigation(
            delta,
            true,
            false,
            false,
            1000.0,
            false,
            0.0,
            1.0,
            KEYS_NONE.0,
            KEYS_NONE.1,
            false,
        );
        assert!(
            (orbit[0] - 0.08).abs() < 1e-6 && (orbit[1] - 0.16).abs() < 1e-6,
            "{orbit:?}"
        );
        assert_eq!(pan, [0.0; 2]);
        // A Shift + primary drag pans exactly like a secondary drag.
        let (orbit, pan, _) = navigation(
            delta,
            true,
            false,
            true,
            1000.0,
            false,
            0.0,
            1.0,
            KEYS_NONE.0,
            KEYS_NONE.1,
            false,
        );
        let (_, secondary, _) = navigation(
            delta,
            false,
            true,
            false,
            1000.0,
            false,
            0.0,
            1.0,
            KEYS_NONE.0,
            KEYS_NONE.1,
            false,
        );
        assert_eq!(orbit, [0.0; 2]);
        assert_eq!(pan, secondary);
    }

    #[test]
    fn the_invert_toggle_flips_only_the_scroll_sign() {
        let scroll = |invert: bool| {
            navigation(
                PLAIN.0,
                PLAIN.1,
                PLAIN.2,
                PLAIN.3,
                1000.0,
                true,
                50.0,
                1.0,
                KEYS_NONE.0,
                KEYS_NONE.1,
                invert,
            )
            .2
        };
        assert!((scroll(false) - 0.1).abs() < 1e-6, "{}", scroll(false));
        assert!((scroll(true) + 0.1).abs() < 1e-6, "{}", scroll(true));
        // Pinch and keys keep their direction under the toggle.
        let pinch = |invert: bool| {
            navigation(
                PLAIN.0,
                PLAIN.1,
                PLAIN.2,
                PLAIN.3,
                1000.0,
                true,
                0.0,
                2.0,
                KEYS_NONE.0,
                KEYS_NONE.1,
                invert,
            )
            .2
        };
        assert!((pinch(false) - std::f32::consts::LN_2).abs() < 1e-6);
        assert!((pinch(true) - std::f32::consts::LN_2).abs() < 1e-6);
        assert!((key_zoom(true, false, false) - 0.25).abs() < 1e-6);
        assert!((key_zoom(true, false, true) - 0.25).abs() < 1e-6);
        assert!(key_zoom(true, true, false).abs() < 1e-6);
        assert!((key_zoom(false, true, false) + 0.25).abs() < 1e-6);
    }

    #[test]
    fn scroll_and_pinch_need_a_hovered_viewport() {
        let (orbit, pan, zoom) = navigation(
            PLAIN.0,
            PLAIN.1,
            PLAIN.2,
            PLAIN.3,
            1000.0,
            false,
            50.0,
            2.0,
            KEYS_NONE.0,
            KEYS_NONE.1,
            false,
        );
        assert_eq!((orbit, pan, zoom), ([0.0; 2], [0.0; 2], 0.0));
    }

    #[test]
    fn navigation_output_stays_inside_the_camera_bounds() {
        let mut camera = Camera::new([0.0; 3], 100.0, 1.0).unwrap();
        let before = camera.distance();
        // A spread pinch (factor 2) halves the distance through the logarithmic path.
        let (orbit, pan, zoom) = navigation(
            PLAIN.0,
            PLAIN.1,
            PLAIN.2,
            PLAIN.3,
            1000.0,
            true,
            0.0,
            2.0,
            KEYS_NONE.0,
            KEYS_NONE.1,
            false,
        );
        camera.navigate(orbit, pan, zoom).unwrap();
        assert!(
            (camera.distance() - before * 0.5).abs() < 1e-3,
            "{}",
            camera.distance()
        );
        // Untrusted gesture factors are dropped, never propagated into the camera.
        for hostile in [0.0, -2.0, f32::NAN, f32::INFINITY, 1.0e30] {
            let (orbit, pan, zoom) = navigation(
                PLAIN.0,
                PLAIN.1,
                PLAIN.2,
                PLAIN.3,
                1000.0,
                true,
                0.0,
                hostile,
                KEYS_NONE.0,
                KEYS_NONE.1,
                false,
            );
            camera.navigate(orbit, pan, zoom).unwrap();
            assert!(camera.distance().is_finite(), "pinch {hostile}");
        }
    }
}
