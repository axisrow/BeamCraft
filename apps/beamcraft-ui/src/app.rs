// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The eframe window: a side panel with the revision, counts, units and adapter, the viewport
//! with orbit (primary drag), pan (Shift + primary drag, or secondary drag), zoom (wheel or
//! trackpad scroll, pinch, `+` and `-`), fit (F), reload (R), click selection and the edit tools
//! (V05.2), the create tools (V05.1), the properties panel and the publication panel of a
//! personal workspace (V06).

use crate::camera::{Camera, View};
use crate::command::{Selected, Selection};
use crate::edit::{EditKind, Edited, Editor, Epoch, Target, ndc_of, ray_through};
use crate::perf::{Observed, PerfRun, Phase, RefreshTiming, Request};
use crate::props::{self, MaterialEdit, Properties, WorkspacePanel};
use crate::render::{Renderer, Uploaded};
use crate::scene::SceneSummary;
use crate::source::{Known, Refreshed, Reloaded};
use crate::stats::STATS;
use crate::tools::{Choice, Created, MAX_TEXT, MemberKind, ToolError, ToolKind, Tools};
use crate::{Error, check_font, command, json_string, source};
use beamcraft_engine::ViewSnapshot;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Instant;

/// Where a `--perf-run` report goes: the JSON object and whether every phase succeeded.
pub type PerfReport = Sender<(String, bool)>;

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
    /// The viewport's MSAA sample count, 1 or 4; `None` is the adapter's default
    /// ([`crate::render::samples_for`]).
    pub samples: Option<u32>,
}

/// The scene on screen: the pinned snapshot, its buffers on the device and what the window
/// needs of the rest (origin, framing, counts). The expanded display buffers are not kept once
/// they are on the device (issue 26, finding 3).
struct Loaded {
    /// Shared with the selection worker, which picks on what the window shows.
    snapshot: Arc<ViewSnapshot>,
    summary: SceneSummary,
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
    /// The scene framing the camera was last fitted to (centre, extent, aspect): a reload that
    /// reports the same framing keeps the user's view instead of fitting again.
    framed: Option<([f32; 3], f32, f32)>,
    editor: Editor,
    edit_job: Option<Receiver<Result<Edited, ToolError>>>,
    select_job: Option<SelectJob>,
    select_epoch: Epoch,
    /// The result of the last select, edit, undo or redo.
    edit_note: Option<Result<String, String>>,
    /// Flips the scroll zoom direction (macOS natural scrolling inverts it); pinch and keys keep theirs.
    invert_zoom: bool,
    /// The element id the running refresh reads the properties of, so a stale refresh never
    /// overwrites a newer selection.
    reload_selection: Option<String>,
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
    /// The `--perf-run` driver and where its report goes, when one runs.
    perf: Option<(PerfRun, PerfReport)>,
    /// Refresh results consumed, whatever their outcome (read by the perf driver).
    reloads_finished: u64,
    /// Commit results consumed, and whether the last one succeeded (read by the perf driver).
    commits_finished: u64,
    last_commit_ok: bool,
    /// Selection results consumed, a stale one too (read by the perf driver).
    selects_finished: u64,
    /// Worker timings of the refreshes consumed while a perf run is active.
    refresh_timings: Vec<RefreshTiming>,
}

/// Records a scene swap in the perf counters: the CPU display bytes and the snapshot mesh bytes
/// the window keeps from now on.
fn note_loaded(loaded: &Loaded) {
    STATS.reloads_applied.add(1);
    // `Loaded` keeps no display buffer.
    STATS.retained_display_bytes.set(0);
    STATS.snapshot_mesh_bytes.set(
        loaded
            .snapshot
            .meshes()
            .iter()
            .map(|m| m.bytes().len() as u64)
            .fold(0, u64::saturating_add),
    );
}

/// A lookup in flight: the epoch it was asked under, its receiver and the flag that gives it up.
/// The answer carries the editor's light selection and the properties panel's full report for
/// the same element.
struct SelectJob {
    asked: u64,
    rx: Receiver<Selection>,
    cancel: Arc<AtomicBool>,
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
    // The label sits left of the list (as with the text fields): drawn below it, it read as
    // the label of the next drop-down.
    ui.horizontal(|ui| {
        ui.label(label);
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
    });
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
    let origin = loaded.summary.origin_mm;
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

/// The offscreen render target size in device pixels: the viewport with its longest side clamped
/// to the renderer's 4096 limit, scaled as a whole. A target that keeps the viewport's aspect
/// ratio can be drawn over the whole panel without stretching the model (a wide window used to
/// widen the picture by the clamped-away part).
fn target_size(size: [f32; 2], scale: f32) -> [u32; 2] {
    let longest = size.iter().fold(0.0f32, |a, v| a.max(*v)) * scale;
    let factor = (4096.0 / longest).min(1.0);
    [
        ((size[0] * scale * factor) as u32).clamp(1, 4096),
        ((size[1] * scale * factor) as u32).clamp(1, 4096),
    ]
}

/// True when the camera should follow the framing of a reloaded scene: nothing was framed yet,
/// or the bounds or the viewport changed. An unchanged scene keeps the user's view, so a Commit
/// or `R` that altered no bounds does not jump back to the whole model.
fn needs_refit(
    framed: Option<([f32; 3], f32, f32)>,
    target: [f32; 3],
    extent: f32,
    aspect: f32,
) -> bool {
    match framed {
        Some((old_target, old_extent, old_aspect)) => {
            old_target != target || old_extent != extent || old_aspect != aspect
        }
        None => true,
    }
}

/// The volume part of a commit note: cubic millimetres below a thousandth of a cubic metre,
/// cubic metres from there, so a small plate does not read as `0.000 m3`.
fn volume_note(volume_mm3: f64) -> String {
    if volume_mm3 < 1e6 {
        format!(", volume {volume_mm3:.0} mm3")
    } else {
        format!(", volume {:.3} m3", volume_mm3 / 1e9)
    }
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
        Self::with_perf_run(cc, source, None)
    }

    /// [`App::new`] that also runs a `--perf-run` plan and sends its report (JSON, success).
    pub fn with_perf_run(
        cc: &eframe::CreationContext<'_>,
        source: Source,
        perf: Option<(Vec<Phase>, PerfReport)>,
    ) -> Result<Self, Error> {
        let state = cc
            .wgpu_render_state
            .clone()
            .ok_or(Error::Input("eframe requires wgpu"))?;
        let text = inter(&cc.egui_ctx);
        let renderer =
            Renderer::with_samples(&state.adapter, &state.device, 1280, 800, source.samples)?;
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
        let perf = perf.map(|(phases, report)| {
            let header = format!(
                "\"adapter\":{},\"device_type\":{},\"backend\":{},\"samples\":{},\"text\":{}",
                json_string(&info.name),
                json_string(&format!("{:?}", info.device_type)),
                json_string(&format!("{:?}", info.backend)),
                renderer.samples,
                text
            );
            (PerfRun::new(phases, header), report)
        });
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
            framed: None,
            editor: Editor::default(),
            edit_job: None,
            select_job: None,
            select_epoch: Epoch::default(),
            edit_note: None,
            invert_zoom: false,
            reload_selection: None,
            selected: None,
            props: props::Props::default(),
            props_job: None,
            props_note: None,
            props_problem: None,
            ws_panel: None,
            ws_job: None,
            ws_note: None,
            mutating: false,
            perf,
            reloads_finished: 0,
            commits_finished: 0,
            last_commit_ok: true,
            selects_finished: 0,
            refresh_timings: Vec::new(),
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
        // Only this refresh replaces `loaded`, so what it knows stays true until it is applied.
        let known = self.loaded.as_ref().map_or_else(Known::default, |l| Known {
            token: Some(l.snapshot.token().clone()),
            resident: l.uploaded.resident(),
        });
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = self.ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("beamcraft-reload".into())
            .spawn(move || {
                let _ = tx.send(source::reload(
                    &cli,
                    &store,
                    for_worker.as_deref(),
                    workspace.as_ref().map(|(p, d)| (p.as_path(), d.as_path())),
                    &known,
                ));
                // The result is only consumed by a UI frame; wake the window for it.
                ctx.request_repaint();
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
        self.reloads_finished += 1;
        if self.perf.is_some() {
            self.refresh_timings.push(reloaded.timing);
        }
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
        let result = reloaded.scene.and_then(|refreshed| match refreshed {
            Refreshed::Unchanged => Ok(None),
            Refreshed::New(new) => {
                let (snapshot, scene) = *new;
                let started = Instant::now();
                // Meshes the device holds are reused, and nothing waits for the device: the
                // queue orders the writes before the next draw.
                let previous = self.loaded.as_ref().map(|l| &l.uploaded);
                let uploaded =
                    Uploaded::upload(&self.state.device, &self.state.queue, &scene, previous)?;
                STATS.upload_ui_ns.add_duration(started.elapsed());
                STATS.uploads.add(1);
                STATS.upload_bytes.add(uploaded.bytes);
                // The display buffers are on the device now: keep the summary, drop the rest.
                Ok(Some(Loaded {
                    snapshot: Arc::new(snapshot),
                    summary: scene.summary(),
                    uploaded,
                }))
            }
        });
        match result {
            // The snapshot on screen again: nothing to build, upload or draw, unless the
            // viewport changed since it was framed.
            Ok(None) => {
                if let Some((target, extent)) = self.loaded.as_ref().map(|l| l.summary.framing()) {
                    let aspect = self.aspect();
                    if needs_refit(self.framed, target, extent, aspect) {
                        match self.camera.fit(target, extent, aspect) {
                            Ok(()) => {
                                self.framed = Some((target, extent, aspect));
                                self.viewport_dirty = true;
                                self.error = None;
                            }
                            Err(e) => self.error = Some(e.to_string()),
                        }
                    } else {
                        self.error = None;
                    }
                }
            }
            Ok(Some(loaded)) => {
                let (target, extent) = loaded.summary.framing();
                let aspect = self.aspect();
                // A scene the camera cannot frame keeps the previous scene and shows why. An
                // unchanged framing keeps the user's camera.
                if needs_refit(self.framed, target, extent, aspect) {
                    match self.camera.fit(target, extent, aspect) {
                        Ok(()) => {
                            self.framed = Some((target, extent, aspect));
                            self.show(loaded);
                        }
                        Err(e) => self.error = Some(e.to_string()),
                    }
                } else {
                    self.show(loaded);
                }
            }
            // A store with no revision yet is a normal state, not an error: the definitions
            // listing treats the same NOT_FOUND the same way.
            Err(e) if source::is_no_revision(&e) => {
                self.loaded = None;
                self.framed = None;
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
        if std::mem::take(&mut self.reload_queued) {
            self.reload();
        }
    }
    /// Puts a refreshed scene on screen. Its image is always drawn again: a scene whose bounds
    /// did not change (an edit inside them, a rename, an undo) can still differ from the last
    /// image.
    fn show(&mut self, loaded: Loaded) {
        note_loaded(&loaded);
        self.loaded = Some(loaded);
        self.error = None;
        self.viewport_dirty = true;
    }

    fn aspect(&self) -> f32 {
        (self.renderer.width as f32 / self.renderer.height as f32).clamp(0.05, 20.0)
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), Error> {
        if width == self.renderer.width && height == self.renderer.height {
            return Ok(());
        }
        let started = Instant::now();
        // Only the targets follow the size; the shader and the pipeline stay.
        self.renderer.resize(&self.state.device, width, height)?;
        self.state
            .renderer
            .write()
            .update_egui_texture_from_wgpu_texture(
                &self.state.device,
                &self.renderer.view,
                wgpu::FilterMode::Linear,
                self.texture,
            );
        self.viewport_dirty = true;
        STATS.resize_ns.add_duration(started.elapsed());
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

    /// Registers a lookup under a new epoch; returns the flag that gives it up.
    fn begin_select(&mut self, rx: Receiver<Selection>) -> Arc<AtomicBool> {
        let cancel = Arc::new(AtomicBool::new(false));
        self.select_job = Some(SelectJob {
            asked: self.select_epoch.advance(),
            rx,
            cancel: Arc::clone(&cancel),
        });
        cancel
    }

    /// Starts a click selection on a worker: the engine's `pick` along the ray, then the details
    /// of the element it hit, the editor's light report and the properties panel's full one from
    /// one lookup ([`command::select`]).
    fn start_select(&mut self, ray: crate::edit::Ray) {
        if self.select_job.is_some() {
            return;
        }
        // The pick runs on the snapshot on screen, so a click selects what the user sees.
        let Some(snapshot) = self.loaded.as_ref().map(|l| Arc::clone(&l.snapshot)) else {
            return;
        };
        let (cli, store) = (self.source.cli.clone(), self.source.store.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = self.ctx.clone();
        let cancel = self.begin_select(rx);
        let spawned = std::thread::Builder::new()
            .name("beamcraft-pick".into())
            .spawn(move || {
                let _ = tx.send(command::select(&cli, &store, &snapshot, &ray, &cancel));
                ctx.request_repaint();
            });
        if spawned.is_err() {
            self.select_job = None;
            self.edit_note = Some(Err("the pick thread could not be started".into()));
        }
    }

    /// Drops any lookup in flight: its result is no longer wanted (Esc, an accepted edit, a
    /// newer lookup), so its CLI process is stopped as well (issue 26, finding 9).
    fn invalidate_select(&mut self) {
        self.select_epoch.advance();
        if let Some(job) = self.select_job.take() {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }

    fn poll_select(&mut self) {
        let Some(SelectJob { asked, rx, .. }) = &self.select_job else {
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
        self.selects_finished += 1;
        if !self.select_epoch.is_current(asked) {
            return;
        }
        let (picked, properties) = result;
        let failed = match &picked {
            Ok(None) => false,
            Ok(Some(selected)) => selected.info.is_err() || !matches!(properties, Ok(Some(_))),
            Err(_) => true,
        };
        if failed {
            STATS.select_errors.add(1);
        }
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
        let ctx = self.ctx.clone();
        self.edit_job = Some(rx);
        self.edit_note = None;
        self.mutating = true;
        let spawned = std::thread::Builder::new()
            .name("beamcraft-edit".into())
            .spawn(move || {
                let _ = tx.send(command::commit_edit(&cli, &edit));
                ctx.request_repaint();
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
        // This lookup replaces any one in flight.
        self.invalidate_select();
        let (cli, store) = (self.source.cli.clone(), self.source.store.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = self.ctx.clone();
        let cancel = self.begin_select(rx);
        let spawned = std::thread::Builder::new()
            .name("beamcraft-info".into())
            .spawn(move || {
                let (info, properties) = command::lookup(&cli, &store, &id, &cancel);
                // An element the revision no longer holds clears the selection; any other
                // failure keeps it selected without details, so the move form is emptied
                // rather than left with the position from before the edit.
                let reply = match info {
                    Err(e) if e.code == "NOT_FOUND" => None,
                    info => Some(Selected {
                        id: id.clone(),
                        info,
                    }),
                };
                let _ = tx.send((Ok(reply), properties));
                ctx.request_repaint();
            });
        if spawned.is_err() {
            self.select_job = None;
        }
    }

    fn edit_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Edit");
        ui.label("Click an element in the view to select it. Esc clears the selection.");
        match self.editor.selected() {
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
        let ctx = self.ctx.clone();
        self.job = Some(rx);
        self.tool_note = None;
        self.mutating = true;
        let spawned = std::thread::Builder::new()
            .name("beamcraft-command".into())
            .spawn(move || {
                let _ = tx.send(command::commit(&cli, &prepared));
                ctx.request_repaint();
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
        self.commits_finished += 1;
        self.last_commit_ok = result.is_ok();
        match result {
            Ok(created) => {
                self.reload();
                let what = match created.kind {
                    ToolKind::Plate => "plate",
                    ToolKind::Member => "member",
                };
                let volume = created.volume_mm3.map_or(String::new(), volume_note);
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

    /// Creates plate number `ordinal` of a perf `edit` phase through the Create panel's own
    /// path: fill the form, preview, commit.
    fn perf_commit(&mut self, ordinal: u32) {
        self.tools.active = ToolKind::Plate;
        let form = &mut self.tools.plate;
        form.width = (100 + ordinal % 1000).to_string();
        form.length = "200".into();
        form.thickness = "10".into();
        form.name = format!("perf-edit-{ordinal}");
        form.material = None;
        let store = self.source.store.clone();
        let started = self.tools.preview(&store).is_ok() && {
            self.start_commit();
            self.job.is_some()
        };
        if !started {
            self.commits_finished += 1;
            self.last_commit_ok = false;
        }
    }

    /// The ray a click straight onto box element number `ordinal` of the loaded snapshot looks
    /// through: from above the centre of its bounds, straight down (box bodies are the ones a
    /// pick can hit).
    fn perf_ray(&self, ordinal: u32) -> Option<crate::edit::Ray> {
        let snapshot = &self.loaded.as_ref()?.snapshot;
        let boxes: Vec<_> = snapshot
            .elements()
            .iter()
            .filter(|e| {
                snapshot
                    .mesh_at(e.full.mesh_slot)
                    .is_some_and(|m| m.vertex_count() == 8 && m.triangle_count() == 12)
            })
            .collect();
        let count = u32::try_from(boxes.len()).ok().filter(|n| *n > 0)?;
        let element = boxes.get(usize::try_from(ordinal % count).ok()?)?;
        let b = element.full.world_bounds;
        Some(crate::edit::Ray {
            origin: [
                (b.min[0] + b.max[0]) / 2.0,
                (b.min[1] + b.max[1]) / 2.0,
                b.max[2] + 1000.0,
            ],
            direction: [0.0, 0.0, -1.0],
        })
    }

    /// One frame of the perf driver, if a run is active.
    fn perf_step(&mut self, ctx: &egui::Context, frame_cpu: Option<f32>) {
        let Some((mut run, report)) = self.perf.take() else {
            return;
        };
        let request: Request = run.step(&Observed {
            now: Instant::now(),
            reloads_finished: self.reloads_finished,
            reload_busy: self.reload_job.is_some() || self.reload_queued,
            commits_finished: self.commits_finished,
            commit_busy: self.job.is_some(),
            last_commit_ok: self.last_commit_ok,
            selects_finished: self.selects_finished,
            select_busy: self.select_job.is_some(),
            target: [self.renderer.width, self.renderer.height],
            frame_cpu,
            refresh_timings: &self.refresh_timings,
        });
        if request.reload {
            self.reload();
        }
        if let Some(ordinal) = request.commit {
            self.perf_commit(ordinal);
        }
        if let Some(ordinal) = request.select {
            match self.perf_ray(ordinal) {
                Some(ray) => self.start_select(ray),
                // Nothing to click on: count it as answered so the run goes on.
                None => self.selects_finished += 1,
            }
        }
        if let Some(ordinal) = request.cancel {
            // What Esc does, then the next click.
            self.invalidate_select();
            self.editor.deselect();
            self.selected = None;
            self.props = props::Props::default();
            match self.perf_ray(ordinal + 1) {
                Some(ray) => self.start_select(ray),
                None => self.selects_finished += 1,
            }
        }
        if request.orbit {
            if let Err(e) = self.camera.navigate([0.01, 0.0], [0.0; 2], 0.0) {
                self.error = Some(e.to_string());
            }
            self.viewport_dirty = true;
        }
        if let Some([width, height]) = request.resize {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(width, height)));
        }
        if request.repaint_now {
            ctx.request_repaint();
        }
        if let Some(after) = request.wake_after {
            ctx.request_repaint_after(after);
        }
        if let Some(finished) = request.finished {
            let _ = report.send(finished);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        self.perf = Some((run, report));
    }

    fn tools_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Create");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tools.active, ToolKind::Plate, "Plate");
            ui.selectable_value(&mut self.tools.active, ToolKind::Member, "Member");
        });
        ui.label("Dimensions in mm.");
        // Borrowed beside the form (disjoint fields), never copied: the panel is laid out in every
        // frame the window draws.
        let definitions = &self.tools.definitions;
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
                    &definitions.materials,
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
                    &definitions.profiles,
                    true,
                );
                choice_combo(
                    ui,
                    "member-material",
                    "Material",
                    &mut form.material,
                    &definitions.materials,
                    false,
                );
                if definitions.profiles.is_empty() {
                    ui.label("The store has no profile definition yet (beamcraft define-profile).");
                }
            }
        }
        let busy = self.tools.busy();
        let mutating = self.mutating;
        let store = &self.source.store;
        // Whether the staged command is still what the form produces; preparing it builds the
        // whole argument list, so it is asked once per frame unless a button acted.
        let mut fresh = false;
        let (mut commit, mut cancelled) = (false, false);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("Preview"))
                .clicked()
            {
                match self.tools.preview(store) {
                    Ok(_) => self.tool_note = None,
                    Err(e) => {
                        self.tools.cancel();
                        self.tool_note = Some(Err(e.to_string()));
                    }
                }
            }
            fresh = self.tools.can_commit(store);
            commit = ui
                .add_enabled(fresh && !mutating, egui::Button::new("Commit"))
                .clicked();
            if ui
                .add_enabled(
                    self.tools.staged().is_some() && !busy,
                    egui::Button::new("Cancel"),
                )
                .clicked()
            {
                self.tools.cancel();
                self.tool_note = Some(Ok("Cancelled; nothing was published.".into()));
                cancelled = true;
            }
        });
        if commit {
            self.start_commit();
        }
        if let Some(staged) = self.tools.staged() {
            let current = if commit || cancelled {
                self.tools.can_commit(&self.source.store)
            } else {
                fresh
            };
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
        let ctx = self.ctx.clone();
        self.props_job = Some(rx);
        self.props_note = None;
        self.mutating = true;
        let spawned = std::thread::Builder::new()
            .name("beamcraft-update".into())
            .spawn(move || {
                let _ = tx.send(command::run_change(&cli, &args));
                ctx.request_repaint();
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
        let ctx = self.ctx.clone();
        self.ws_job = Some(rx);
        self.ws_note = None;
        self.mutating = true;
        let spawned = std::thread::Builder::new()
            .name("beamcraft-publish".into())
            .spawn(move || {
                let _ = tx.send(command::run_change(&cli, &args));
                ctx.request_repaint();
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
        // Borrowed, never copied: the panel is laid out in every frame the window draws.
        let Some(selected) = &self.selected else {
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
        let materials = &self.tools.definitions.materials;
        let store = &self.source.store;
        let busy = self.props.busy() || self.mutating;
        let mutating = self.mutating;
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
                    for choice in materials {
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
        // As in the Create panel: the staged patch is prepared once per frame unless a button
        // acted.
        let mut fresh = false;
        let (mut commit, mut cancelled) = (false, false);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("Preview"))
                .clicked()
            {
                match self.props.preview(store, &selected.element_id) {
                    Ok(()) => self.props_note = None,
                    Err(e) => {
                        self.props.cancel();
                        self.props_note = Some(Err(e.to_string()));
                    }
                }
            }
            fresh = self.props.can_commit(store, &selected.element_id);
            commit = ui
                .add_enabled(fresh && !mutating, egui::Button::new("Commit"))
                .clicked();
            if ui
                .add_enabled(
                    self.props.staged().is_some() && !busy,
                    egui::Button::new("Cancel"),
                )
                .clicked()
            {
                self.props.cancel();
                self.props_note = Some(Ok("Cancelled; nothing was published.".into()));
                cancelled = true;
            }
        });
        if commit {
            self.start_update();
        }
        if let Some(staged) = self.props.staged() {
            let current = if commit || cancelled {
                self.selected
                    .as_ref()
                    .is_some_and(|s| self.props.can_commit(&self.source.store, &s.element_id))
            } else {
                fresh
            };
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
                // Owned, so the button closure never holds a panel borrow.
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

    fn info(&mut self, ui: &mut egui::Ui) {
        ui.heading("BeamCraft");
        ui.label(format!("{:.0} viewport redraws/s", self.fps));
        ui.label(&self.adapter);
        ui.label(format!("{}x MSAA", self.renderer.samples));
        ui.separator();
        match &self.loaded {
            Some(l) => {
                let pin = l.snapshot.pin();
                let (meshes, instances) = l.summary.counts();
                let revision = pin.revision().to_string();
                ui.label(format!(
                    "Revision {}… generation {}",
                    revision.get(..12).unwrap_or(&revision),
                    pin.generation()
                ));
                ui.label(format!("Elements: {}", l.summary.elements));
                ui.label(format!("Meshes: {meshes} (instances: {instances})"));
                ui.label(format!("Triangles: {}", l.summary.triangles));
                let size = [0, 1, 2].map(|a| {
                    f64::from(l.summary.max.get(a).copied().unwrap_or(0.0))
                        - f64::from(l.summary.min.get(a).copied().unwrap_or(0.0))
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
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        STATS.frames.add(1);
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
        self.poll_update();
        self.poll_workspace();
        self.poll_select();
        self.poll_reload();
        self.perf_step(ui.ctx(), frame.info().cpu_usage);
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
            let (target, extent) = l.summary.framing();
            if let Err(e) = self.camera.fit(target, extent, self.aspect()) {
                self.error = Some(e.to_string());
            }
            self.viewport_dirty = true;
        }
        if self.text {
            // A fixed width: the 3D view must not shift sideways when a tab or a chosen
            // definition name is wider, and the coordinate rows must fit at the default size.
            egui::Panel::left("info")
                .exact_size(320.0)
                .show(ui, |ui| self.panel(ui));
        }
        egui::CentralPanel::default().show(ui, |ui| {
            let size = ui.available_size();
            let scale = ui.ctx().pixels_per_point();
            let [width, height] = target_size([size.x, size.y], scale);
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
                            && let Some(ray) = ray_through(&view, ndc, l.summary.origin_mm)
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
    use super::{App, Source, navigation, needs_refit, target_size, volume_note};
    use crate::Error;
    use crate::camera::Camera;
    use crate::perf::RefreshTiming;
    use crate::render::Gpu;
    use crate::scene::Scene;
    use crate::source::{Refreshed, Reloaded};
    use crate::tools::ToolError;
    use beamcraft_engine::synthetic::plate_scene;
    use beamcraft_engine::{Limits, Representation, ViewPin, ViewSnapshot};
    use beamcraft_model::digest::Sha256;
    use eframe::egui_wgpu;
    use std::sync::atomic::Ordering;

    /// The window state on the headless adapter, with no window and no CLI behind it: the
    /// refresh it starts fails and is never read, and a test hands it results itself. `None`,
    /// saying so, without a native adapter.
    fn headless() -> Option<App> {
        let gpu = match pollster::block_on(Gpu::headless()) {
            Ok(gpu) => gpu,
            Err(Error::NoAdapter) => {
                eprintln!("SKIP window test: no native wgpu adapter");
                return None;
            }
            Err(e) => panic!("adapter or device failed: {e}"),
        };
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let renderer =
            egui_wgpu::Renderer::new(&gpu.device, format, egui_wgpu::RendererOptions::default());
        // eframe's own constructor for windowless tests (doc-hidden; eframe is pinned).
        let mut cc = eframe::CreationContext::_new_kittest(egui::Context::default());
        cc.wgpu_render_state = Some(egui_wgpu::RenderState {
            adapter: gpu.adapter,
            available_adapters: Vec::new(),
            instance: wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle()),
            device: gpu.device,
            queue: gpu.queue,
            target_format: format,
            renderer: std::sync::Arc::new(egui::mutex::RwLock::new(renderer)),
            surface_config: egui_wgpu::SurfaceConfig::LOW_LATENCY,
        });
        let missing = std::env::temp_dir().join("beamcraft-window-test-no-cli");
        let source = Source {
            store: missing.clone(),
            cli: missing,
            workspace: None,
            published: None,
            samples: None,
        };
        Some(App::new(&cc, source).unwrap())
    }

    /// A refresh that read `scene`.
    fn reloaded(scene: Result<Refreshed, Error>) -> Reloaded {
        Reloaded {
            scene,
            definitions: Err(ToolError::local("NOT_FOUND", "no store in this test")),
            properties: None,
            workspace: None,
            timing: RefreshTiming::default(),
        }
    }

    /// A refresh whose scene is `plates` synthetic kernel plates.
    fn refreshed(plates: u32) -> Reloaded {
        let pin = ViewPin::new(Sha256::digest(b"revision").unwrap(), 1);
        let snapshot =
            ViewSnapshot::build(pin, plate_scene(plates, false).unwrap(), &Limits::default())
                .unwrap();
        let scene = Scene::from_snapshot(&snapshot, Representation::Full).unwrap();
        reloaded(Ok(Refreshed::New(Box::new((snapshot, scene)))))
    }

    /// Hands `reloaded` to the window as the result of the refresh in flight.
    fn deliver(app: &mut App, reloaded: Reloaded) {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(reloaded).unwrap();
        app.reload_job = Some(rx);
        app.poll_reload();
    }

    #[test]
    fn a_lookup_that_is_given_up_is_told_to_stop() {
        let Some(mut app) = headless() else { return };
        let (_tx, rx) = std::sync::mpsc::channel();
        let cancel = app.begin_select(rx);
        assert!(!cancel.load(Ordering::Relaxed));
        // What Esc, an accepted edit and a newer lookup do.
        app.invalidate_select();
        assert!(cancel.load(Ordering::Relaxed), "its CLI process is stopped");
        assert!(app.select_job.is_none());
    }

    #[test]
    fn the_snapshot_on_screen_again_is_neither_uploaded_nor_drawn() {
        let Some(mut app) = headless() else { return };
        deliver(&mut app, refreshed(2));
        app.viewport_dirty = false;
        let token = app.loaded.as_ref().map(|l| l.snapshot.token().clone());
        deliver(&mut app, reloaded(Ok(Refreshed::Unchanged)));
        assert_eq!(
            app.loaded.as_ref().map(|l| l.snapshot.token().clone()),
            token
        );
        assert!(
            !app.viewport_dirty,
            "the image on screen is still the scene's"
        );
        // The scene keeps no display buffer once it is on the device.
        assert!(app.loaded.is_some());
    }

    #[test]
    fn a_refreshed_scene_is_drawn_again_even_when_its_framing_is_unchanged() {
        let Some(mut app) = headless() else { return };
        deliver(&mut app, refreshed(2));
        assert!(app.loaded.is_some() && app.viewport_dirty);
        let framed = app.framed;
        // What a frame does after drawing the image.
        app.viewport_dirty = false;
        // The same bounds again, as after an edit inside them: the camera stays where the user
        // left it, and the new scene is still drawn.
        deliver(&mut app, refreshed(2));
        assert_eq!(app.framed, framed);
        assert!(
            app.viewport_dirty,
            "the new scene must replace the old image"
        );
    }

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

    #[test]
    fn the_render_target_is_clamped_as_a_whole() {
        // Below the limit the viewport passes through unchanged.
        assert_eq!(target_size([1280.0, 800.0], 1.0), [1280, 800]);
        // A 5400 px window: both sides shrink by the same factor, so a circle stays a circle.
        let [w, h] = target_size([5400.0, 900.0], 1.0);
        assert_eq!(w, 4096);
        assert!((h as f32 / 900.0 - 4096.0 / 5400.0).abs() < 0.01);
        // Simulated Retina (2x) reaches the limit at half the point size.
        let [w, h] = target_size([2560.0, 1300.0], 2.0);
        assert_eq!((w, h), (4096, 2080));
        // A degenerate viewport still yields a one-pixel target.
        assert_eq!(target_size([0.0, 0.0], 1.0), [1, 1]);
    }

    #[test]
    fn an_unchanged_framing_keeps_the_camera() {
        let framing = ([10.0, 20.0, 30.0], 500.0, 1.6);
        assert!(needs_refit(None, framing.0, framing.1, framing.2));
        assert!(!needs_refit(Some(framing), framing.0, framing.1, framing.2));
        // New bounds or a new viewport aspect follow.
        assert!(needs_refit(
            Some(framing),
            [0.0, 20.0, 30.0],
            framing.1,
            framing.2
        ));
        assert!(needs_refit(Some(framing), framing.0, 600.0, framing.2));
        assert!(needs_refit(Some(framing), framing.0, framing.1, 1.0));
    }

    #[test]
    fn a_small_volume_is_not_zero() {
        assert_eq!(volume_note(400_000.0), ", volume 400000 mm3");
        assert_eq!(volume_note(999_999.0), ", volume 999999 mm3");
        assert_eq!(volume_note(1e6), ", volume 0.001 m3");
        assert_eq!(volume_note(1.025e9), ", volume 1.025 m3");
    }
}
