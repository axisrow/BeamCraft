// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The eframe window: a side panel with the revision, counts, units and adapter, and the viewport
//! navigated through one of two input presets: `Mouse` (the default: wheel-button drag pans,
//! Ctrl + wheel-button drag orbits, the left button stays free for selection) or `Trackpad`
//! (primary drag orbits, Shift + primary drag pans, pinch zooms). In both the wheel zooms toward
//! the cursor, F fits and R reloads.

mod workbench;

use crate::camera::{Camera, View};
use crate::command::Selected;
use crate::edit::{
    EditKind, Edited, Editor, Epoch, Target, ground_point, ndc_of, pick_point_in, ray_through,
};
use crate::perf::{Observed, PerfRun, Phase as PerfPhase, RefreshTiming, Request};
use crate::props::{Properties, State};
use crate::publish::{Phase, Published, Publisher, Status};
use crate::render::{Renderer, Uploaded};
use crate::scene::{GhostMesh, SceneSummary};
use crate::source::{Known, Refreshed, Reloaded};
use crate::stats::Stats;
use crate::tools::{BeamPlace, Choice, Created, MAX_TEXT, ToolError, ToolKind, Tools};
use crate::{Error, check_font, command, json_string, source};
use beamcraft_engine::ViewSnapshot;
use beamcraft_engine::element::ElementId;
use beamcraft_engine::member::{AuthoredMember, ProfileInfo};
use beamcraft_model::command::{MemberKind as ModelMemberKind, Placement};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

/// Where a `--perf-run` report goes: the JSON object and whether every phase succeeded.
pub type PerfReport = Sender<(String, bool)>;

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
    /// The published store the workspace was forked from (`--published`), the target of Publish.
    pub published: Option<PathBuf>,
    /// The viewport's MSAA sample count, 1 or 4; `None` is the adapter's default
    /// ([`crate::render::samples_for`]).
    pub samples: Option<u32>,
    /// The author identity (UUIDv7) recorded as the actor of the window's edits; `None` lets the
    /// CLI make one per command.
    pub actor: Option<String>,
    /// The environment chosen at start, shown in the header.
    pub environment: Option<String>,
    /// The author's name, shown in the header.
    pub author: Option<String>,
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
    source: Source,
    renderer: Renderer,
    texture: egui::TextureId,
    /// The counters of the work this window does, shared with its worker threads (see
    /// [`crate::stats`]). One per window, so tests never count into each other.
    stats: Arc<Stats>,
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
    /// The store watcher: one message per head change another process committed, so the view
    /// refreshes without the `R` key. `None` when the watcher thread could not start.
    store_watch: Option<Receiver<()>>,
    /// The watcher's shutdown handle: dropping it (with the window) ends the thread even
    /// when the store never changes again. Never read: its drop is the shutdown.
    #[expect(dead_code)]
    store_watch_shutdown: Option<Sender<()>>,
    /// The result of the last Preview, Cancel or Commit.
    tool_note: Option<Result<String, String>>,
    /// The two-click 3D placement of the member tool (issue #134).
    beam: BeamPlace,
    /// Whether viewport clicks place the beam instead of selecting.
    beam_mode: bool,
    /// The millimetre-quantized hover end the current ghost was built for: the buffers rebuild
    /// only when the preview actually moves.
    ghost_end: Option<[i64; 3]>,
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
    /// The active viewport input binding preset; `Mouse` is the default.
    preset: Preset,
    props: Properties,
    props_job: Option<Receiver<Result<Edited, ToolError>>>,
    /// The result of the last property Preview, Cancel or Commit.
    props_note: Option<Result<String, String>>,
    publisher: Publisher,
    publish_job: Option<Receiver<Result<Published, ToolError>>>,
    status_job: Option<Receiver<Result<Status, ToolError>>>,
    status_queued: bool,
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
    /// The tool chosen on the ribbon; the inspector shows what it needs.
    bench: workbench::Bench,
    /// Whether the Details window is open.
    details: bool,
    /// A zoom (true: in) or fit asked for by the view buttons, applied in the next frame.
    pending_zoom: Option<bool>,
    pending_fit: bool,
    logo: Option<egui::TextureHandle>,
    logo_tried: bool,
}

/// Records a scene swap in the perf counters: the CPU display bytes and the snapshot mesh bytes
/// the window keeps from now on.
fn note_loaded(stats: &Stats, loaded: &Loaded) {
    stats.reloads_applied.add(1);
    // `Loaded` keeps no display buffer.
    stats.retained_display_bytes.set(0);
    stats.snapshot_mesh_bytes.set(
        loaded
            .snapshot
            .meshes()
            .iter()
            .map(|m| m.bytes().len() as u64)
            .fold(0, u64::saturating_add),
    );
}

/// A lookup in flight: the epoch it was asked under, its receiver and the flag that gives it up.
struct SelectJob {
    asked: u64,
    rx: Receiver<Result<Option<Selected>, ToolError>>,
    cancel: Arc<AtomicBool>,
}

/// The viewport input binding preset. `Mouse` keeps the left button free for selection: the
/// camera moves with the wheel button only. `Trackpad` is the laptop mapping for users without
/// a wheel: primary drag orbits, Shift + primary drag or secondary drag pans, pinch zooms.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Preset {
    /// Default mouse bindings: wheel zooms toward the cursor, wheel-button drag pans,
    /// Ctrl + wheel-button drag orbits, and the left button never moves the camera.
    #[default]
    Mouse,
    /// Trackpad (macOS) bindings, selected in the side panel.
    Trackpad,
}

/// One frame of viewport navigation in `Camera::navigate` argument space: orbit radians, a
/// screen-relative pan in millimetres and logarithmic zoom, mapped by the active `preset`.
/// `Mouse`: middle drag pans, Ctrl + middle drag orbits, a primary drag never moves the camera.
/// `Trackpad`: Shift + primary drag or secondary drag pans, primary drag orbits. The scroll
/// zoom sign follows `invert_zoom`; pinch zoom and the `+`/`-` keys keep their direction.
/// Scroll and pinch only count while the viewport is hovered. A hostile pinch factor is
/// dropped, not propagated, so what comes back is always finite and inside what `navigate`
/// accepts.
#[allow(clippy::too_many_arguments)] // one flat mapping from one frame of input to navigate's arguments
fn navigation(
    preset: Preset,
    pointer_delta: egui::Vec2,
    primary: bool,
    secondary: bool,
    middle: bool,
    ctrl: bool,
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
    let orbit = [pointer_delta.x * 0.008, -pointer_delta.y * 0.008];
    let (orbit, pan) = match preset {
        // The left button is reserved for selection: a primary drag never moves the camera.
        Preset::Mouse => {
            if middle && ctrl {
                (orbit, [0.0; 2])
            } else if middle {
                ([0.0; 2], pan)
            } else {
                ([0.0; 2], [0.0; 2])
            }
        }
        Preset::Trackpad => {
            if (primary && shift) || secondary {
                ([0.0; 2], pan)
            } else if primary {
                (orbit, [0.0; 2])
            } else {
                ([0.0; 2], [0.0; 2])
            }
        }
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
        .map(|digest| {
            choices.iter().find(|c| &c.digest == digest).map_or_else(
                || format!("unlisted definition {}", digest.get(..8).unwrap_or(digest)),
                |c| c.label.clone(),
            )
        })
        .unwrap_or_else(|| if required { "choose…" } else { "none" }.to_owned());
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

/// How often the window checks the store for revisions another process has committed.
const WATCH_INTERVAL: Duration = Duration::from_millis(500);

/// The token of a store's committed revisions: one record file per committed write, so the
/// entry count and the newest file name together identify the head without trusting
/// filesystem timestamp resolution. A missing directory is "no signal yet", not an error:
/// the view-export reports the state of an empty store itself.
fn revisions_token(store: &Path) -> Option<(usize, String)> {
    let mut newest = String::new();
    let mut count = 0usize;
    for entry in std::fs::read_dir(store.join("revisions")).ok()? {
        let entry = entry.ok()?;
        count += 1;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name > newest {
            newest = name;
        }
    }
    Some((count, newest))
}

/// Watches a store and reports every change of its revisions as one message, so the window
/// refreshes by itself when another process (the CLI, an agent) commits to the store it
/// shows. Returns the signal receiver, the shutdown handle and the thread handle: the
/// watcher exits when the shutdown handle is dropped (the window's lifetime), whether or
/// not the store ever changes. `None` when the thread could not start: the `R` key keeps
/// working either way.
fn spawn_watch(store: PathBuf) -> Option<(Receiver<()>, Sender<()>, std::thread::JoinHandle<()>)> {
    let (tx, rx) = std::sync::mpsc::channel();
    let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("beamcraft-watch".into())
        .spawn(move || {
            let mut seen = revisions_token(&store);
            loop {
                match shutdown_rx.recv_timeout(WATCH_INTERVAL) {
                    // The window dropped the handle (or asked to stop): exit without
                    // touching the filesystem again.
                    Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                }
                let now = revisions_token(&store);
                if now != seen {
                    seen = now;
                    if tx.send(()).is_err() {
                        return; // the window is gone
                    }
                }
            }
        })
        .ok()?;
    Some((rx, shutdown_tx, spawned))
}

/// One resolved world point, for notes and hints.
struct PointText([f64; 3]);
impl std::fmt::Display for PointText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [x, y, z] = self.0;
        write!(f, "({x:.1}, {y:.1}, {z:.1}) mm")
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
pub(crate) fn inter(ctx: &egui::Context) -> bool {
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
    let bold = crate::theme::bold_family();
    match bytes {
        Some(Ok(bytes)) => {
            let mut fonts = egui::FontDefinitions::empty();
            fonts
                .font_data
                .insert("Inter".into(), egui::FontData::from_owned(bytes).into());
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.insert(family, vec!["Inter".into()]);
            }
            // Inter Bold beside the regular face, checked the same way; headings fall back to
            // the regular face without it.
            let mut bold_fonts = vec!["Inter".to_owned()];
            if let Some(bytes) = candidates
                .iter()
                .find(|p| p.is_file())
                .and_then(|regular| bounded_font(&regular.with_file_name("Inter-Bold.ttf")))
            {
                fonts.font_data.insert(
                    "Inter-Bold".into(),
                    egui::FontData::from_owned(bytes).into(),
                );
                bold_fonts.insert(0, "Inter-Bold".into());
            }
            fonts.families.insert(bold, bold_fonts);
            ctx.set_fonts(fonts);
            true
        }
        _ => {
            eprintln!("Inter is unavailable; the window runs without text.");
            let mut fonts = egui::FontDefinitions::empty();
            fonts.families.insert(bold, Vec::new());
            ctx.set_fonts(fonts);
            false
        }
    }
}

/// The bytes of a font file of at most 4 MiB that egui can load, or `None`.
fn bounded_font(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.is_empty() || bytes.len() > 4 * 1024 * 1024 || check_font(&bytes).is_err() {
        return None;
    }
    Some(bytes)
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

/// The window/application icon: the official logo on a rounded tile, the same picture as the
/// macOS ICNS, the Windows ICO and the Linux launcher icon (`assets/icons/beamcraft-icon.png`
/// next to the program or in the source tree; in the macOS bundle the grid-margin variant
/// `Contents/Resources/beamcraft-icon.png`, which eframe also applies to the Dock), averaged down to 256 x 256. When it
/// cannot be read, the coded [`icon`] is used, so no asset bytes are compiled into the binary.
pub fn load_icon() -> egui::IconData {
    const SIDE: u32 = 256;
    match crate::theme::app_icon_rgba(SIDE as usize) {
        Some(rgba) => egui::IconData {
            rgba,
            width: SIDE,
            height: SIDE,
        },
        None => icon(),
    }
}

impl App {
    /// Creates the window state and loads the accepted snapshot.
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        source: Source,
        stats: Arc<Stats>,
    ) -> Result<Self, Error> {
        Self::with_perf_run(cc, source, None, stats)
    }

    /// [`App::new`] that also runs a `--perf-run` plan and sends its report (JSON, success).
    pub fn with_perf_run(
        cc: &eframe::CreationContext<'_>,
        source: Source,
        perf: Option<(Vec<PerfPhase>, PerfReport)>,
        stats: Arc<Stats>,
    ) -> Result<Self, Error> {
        Self::open(
            &cc.egui_ctx,
            cc.wgpu_render_state.as_ref(),
            source,
            perf,
            stats,
        )
    }

    /// [`App::with_perf_run`] from a running window's context and render state, so a window that
    /// started on the start page can open a model later.
    pub fn open(
        ctx: &egui::Context,
        state: Option<&eframe::egui_wgpu::RenderState>,
        source: Source,
        perf: Option<(Vec<PerfPhase>, PerfReport)>,
        stats: Arc<Stats>,
    ) -> Result<Self, Error> {
        let state = state.cloned().ok_or(Error::Input("eframe requires wgpu"))?;
        let text = inter(ctx);
        crate::theme::install(ctx);
        let renderer = Renderer::with_samples(
            &state.adapter,
            &state.device,
            1280,
            800,
            source.samples,
            &stats,
        )?;
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
        let publisher = Publisher::new(source.published.clone(), source.workspace.clone());
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
        let (store_watch, store_watch_shutdown) = match spawn_watch(source.store.clone()) {
            Some((rx, shutdown, handle)) => {
                drop(handle);
                (Some(rx), Some(shutdown))
            }
            None => (None, None),
        };
        let mut app = Self {
            state,
            source,
            renderer,
            texture,
            stats,
            camera: Camera::new([0.0; 3], 1000.0, 1.6)?,
            loaded: None,
            error: None,
            text,
            adapter,
            last: Instant::now(),
            fps: 0.0,
            tools: Tools::default(),
            beam: BeamPlace::default(),
            beam_mode: false,
            ghost_end: None,
            job: None,
            reload_job: None,
            reload_queued: false,
            store_watch,
            store_watch_shutdown,
            tool_note: None,
            refresh_error: None,
            framed: None,
            editor: Editor::default(),
            edit_job: None,
            select_job: None,
            select_epoch: Epoch::default(),
            edit_note: None,
            invert_zoom: false,
            preset: Preset::default(),
            props: Properties::default(),
            props_job: None,
            props_note: None,
            publisher,
            publish_job: None,
            status_job: None,
            status_queued: false,
            perf,
            reloads_finished: 0,
            commits_finished: 0,
            last_commit_ok: true,
            selects_finished: 0,
            refresh_timings: Vec::new(),
            bench: workbench::Bench::default(),
            details: false,
            pending_zoom: None,
            pending_fit: false,
            logo: None,
            logo_tried: false,
        };
        app.tools.actor.clone_from(&app.source.actor);
        app.editor.actor.clone_from(&app.source.actor);
        app.props.actor.clone_from(&app.source.actor);
        app.reload();
        app.refresh_status();
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
        // Only this refresh replaces `loaded`, so what it knows stays true until it is applied.
        let known = self.loaded.as_ref().map_or_else(Known::default, |l| Known {
            token: Some(l.snapshot.token().clone()),
            resident: l.uploaded.resident(),
        });
        let (tx, rx) = std::sync::mpsc::channel();
        let stats = Arc::clone(&self.stats);
        let spawned = std::thread::Builder::new()
            .name("beamcraft-reload".into())
            .spawn(move || {
                let _ = tx.send(source::reload(&cli, &store, &known, &stats));
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
        // The listing carries the head it was read from; an unreadable listing leaves it unknown.
        self.props.observe_head(
            reloaded
                .definitions
                .as_ref()
                .ok()
                .and_then(|d| d.head.clone()),
        );
        self.reloads_finished += 1;
        if self.perf.is_some() {
            self.refresh_timings.push(reloaded.timing);
        }
        self.refresh_error = self.tools.apply_definitions(reloaded.definitions);
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
                self.stats.upload_ui_ns.add_duration(started.elapsed());
                self.stats.uploads.add(1);
                self.stats.upload_bytes.add(uploaded.bytes);
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
    /// Puts a refreshed scene on screen. Its image is drawn again either way (the window draws
    /// every frame): a scene whose bounds did not change (an edit inside them, a rename, an undo)
    /// can still differ from the last image.
    fn show(&mut self, loaded: Loaded) {
        note_loaded(&self.stats, &loaded);
        self.loaded = Some(loaded);
        self.error = None;
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
        self.renderer
            .resize(&self.state.device, width, height, &self.stats)?;
        self.state
            .renderer
            .write()
            .update_egui_texture_from_wgpu_texture(
                &self.state.device,
                &self.renderer.view,
                wgpu::FilterMode::Linear,
                self.texture,
            );
        self.stats.resize_ns.add_duration(started.elapsed());
        Ok(())
    }

    fn target(&self) -> Target<'_> {
        Target {
            store: &self.source.store,
            workspace: self.source.workspace.as_deref(),
        }
    }

    /// Registers a lookup under a new epoch; returns the flag that gives it up.
    fn begin_select(
        &mut self,
        rx: Receiver<Result<Option<Selected>, ToolError>>,
    ) -> Arc<AtomicBool> {
        let cancel = Arc::new(AtomicBool::new(false));
        self.select_job = Some(SelectJob {
            asked: self.select_epoch.advance(),
            rx,
            cancel: Arc::clone(&cancel),
        });
        cancel
    }

    /// Starts a click selection on a worker: the engine's `pick` along the ray, then the details
    /// of the element it hit, from one lookup ([`command::select`]) on the snapshot on screen.
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
        let cancel = self.begin_select(rx);
        let stats = Arc::clone(&self.stats);
        let spawned = std::thread::Builder::new()
            .name("beamcraft-pick".into())
            .spawn(move || {
                let _ = tx.send(command::select(
                    &cli, &store, &snapshot, &ray, &cancel, &stats,
                ));
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
            Err(TryRecvError::Disconnected) => Err(ToolError {
                code: "CLI_FAILED".into(),
                message: "the pick thread ended without a result".into(),
            }),
        };
        self.select_job = None;
        self.selects_finished += 1;
        if !self.select_epoch.is_current(asked) {
            return;
        }
        let failed = match &result {
            Ok(None) => false,
            Ok(Some(selected)) => selected.info.is_err() || selected.details.is_err(),
            Err(_) => true,
        };
        if failed {
            self.stats.select_errors.add(1);
        }
        match result {
            Ok(None) => {
                self.editor.deselect();
                self.props.clear();
                self.edit_note = None;
            }
            Ok(Some(Selected { id, info, details })) => {
                match info {
                    Ok(info) => {
                        self.editor.select(info);
                        self.edit_note = None;
                    }
                    Err(e) => {
                        self.editor.select_id(id.clone());
                        self.edit_note = Some(Err(format!("details: {e}")));
                    }
                }
                if self.props.selected() != Some(id.as_str()) {
                    self.props_note = None;
                }
                self.props.load(&id, details);
            }
            Err(e) => self.edit_note = Some(Err(e.to_string())),
        }
    }

    fn start_edit(&mut self, direct: Option<EditKind>) {
        if self.other_command_running() {
            self.edit_note = Some(Err(
                "another command is running; wait for it to finish.".into()
            ));
            return;
        }
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
        let stats = Arc::clone(&self.stats);
        let spawned = std::thread::Builder::new()
            .name("beamcraft-edit".into())
            .spawn(move || {
                let _ = tx.send(command::commit_edit(&cli, &edit, &stats));
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
                    EditKind::Delete => {
                        self.editor.deselect();
                        self.props.clear();
                    }
                    // The element may have moved, or the undo may have removed it: ask again.
                    _ => self.refresh_selection(),
                }
                self.refresh_status();
            }
            Err(e) => {
                if e.code == "OUTCOME_UNKNOWN" {
                    self.invalidate_select();
                    self.reload();
                    self.refresh_selection();
                    self.refresh_status();
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
        let cancel = self.begin_select(rx);
        let stats = Arc::clone(&self.stats);
        let spawned = std::thread::Builder::new()
            .name("beamcraft-info".into())
            .spawn(move || {
                let (info, details) = command::lookup(&cli, &store, &id, &cancel, &stats);
                // An element the revision no longer holds clears the selection; any other
                // failure keeps it selected without details, so the move form is emptied
                // rather than left with the position from before the edit.
                let reply = match info {
                    Err(e) if e.code == "NOT_FOUND" => None,
                    info => Some(Selected {
                        id: id.clone(),
                        info,
                        details,
                    }),
                };
                let _ = tx.send(Ok(reply));
            });
        if spawned.is_err() {
            self.select_job = None;
        }
    }

    /// True while a command that writes to the store (or publishes it) is running: the property,
    /// publish and edit actions never run at the same time.
    fn other_command_running(&self) -> bool {
        self.props_job.is_some()
            || self.publish_job.is_some()
            || self.edit_job.is_some()
            || self.job.is_some()
    }

    /// Starts the committed property edit on a worker: the same `update-element` the CLI runs.
    fn start_property_commit(&mut self) {
        if self.other_command_running() {
            self.props_note = Some(Err(
                "another command is running; wait for it to finish.".into()
            ));
            return;
        }
        let Some(edit) = self.props.begin_commit(&self.source.store) else {
            return;
        };
        let cli = self.source.cli.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.props_job = Some(rx);
        self.props_note = None;
        let stats = Arc::clone(&self.stats);
        let spawned = std::thread::Builder::new()
            .name("beamcraft-properties".into())
            .spawn(move || {
                let _ = tx.send(command::commit_property(&cli, &edit, &stats));
            });
        if spawned.is_err() {
            self.props_job = None;
            self.props.finish();
            self.props_note = Some(Err("the command thread could not be started".into()));
        }
    }

    fn poll_props(&mut self) {
        let Some(rx) = &self.props_job else { return };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(ToolError {
                code: "CLI_FAILED".into(),
                message: "the command thread ended without a result".into(),
            }),
        };
        self.props_job = None;
        self.props.finish();
        match result {
            Ok(done) => {
                self.invalidate_select();
                self.reload();
                self.props_note = Some(Ok(format!(
                    "Updated {} (accepted revision {}, sequence {}).",
                    done.element_id.as_deref().unwrap_or("the element"),
                    done.manifest_digest
                        .get(..12)
                        .unwrap_or(&done.manifest_digest),
                    done.sequence
                )));
                self.refresh_selection();
                self.refresh_status();
            }
            Err(e) => {
                if e.code == "OUTCOME_UNKNOWN" {
                    self.invalidate_select();
                    self.reload();
                    self.refresh_selection();
                    self.refresh_status();
                }
                self.props_note = Some(Err(e.to_string()));
            }
        }
    }

    /// Reads the workspace status on a worker when the window can publish.
    fn refresh_status(&mut self) {
        let Ok((published, workspace)) = self.publisher.target() else {
            return;
        };
        if self.status_job.is_some() {
            self.status_queued = true;
            return;
        }
        let (published, workspace) = (published.to_path_buf(), workspace.to_path_buf());
        let cli = self.source.cli.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let stats = Arc::clone(&self.stats);
        let spawned = std::thread::Builder::new()
            .name("beamcraft-status".into())
            .spawn(move || {
                let _ = tx.send(command::workspace_status(
                    &cli, &published, &workspace, &stats,
                ));
            });
        if spawned.is_ok() {
            self.status_job = Some(rx);
        }
    }

    fn poll_status(&mut self) {
        let Some(rx) = &self.status_job else { return };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(ToolError {
                code: "CLI_FAILED".into(),
                message: "the status thread ended without a result".into(),
            }),
        };
        self.status_job = None;
        self.publisher.set_status(result);
        if std::mem::take(&mut self.status_queued) {
            self.refresh_status();
        }
    }

    /// Runs the confirmed `workspace-publish` on a worker.
    fn start_publish(&mut self) {
        if self.other_command_running() {
            self.publisher.cancel();
            self.publisher.note = Some(Err(
                "another command is running; wait for it to finish.".into()
            ));
            return;
        }
        let Some(args) = self.publisher.confirm() else {
            return;
        };
        let cli = self.source.cli.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.publish_job = Some(rx);
        let stats = Arc::clone(&self.stats);
        let spawned = std::thread::Builder::new()
            .name("beamcraft-publish".into())
            .spawn(move || {
                let _ = tx.send(command::publish(&cli, &args, &stats));
            });
        if spawned.is_err() {
            self.publish_job = None;
            self.publisher.finish(Err(ToolError::local(
                "CLI_UNAVAILABLE",
                "the publish thread could not be started",
            )));
        }
    }

    fn poll_publish(&mut self) {
        let Some(rx) = &self.publish_job else { return };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(ToolError {
                code: "CLI_FAILED".into(),
                message: "the publish thread ended without a result".into(),
            }),
        };
        self.publish_job = None;
        self.publisher.finish(result);
        self.refresh_status();
    }

    /// Re-reads the selected element and the head, so a stale reading becomes current.
    fn reread(&mut self) {
        self.invalidate_select();
        self.reload();
        self.refresh_selection();
    }

    fn props_panel(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.props.selected().map(str::to_owned) else {
            ui.label("Select a plate or straight member to see its properties.");
            return;
        };
        ui.label(format!("Element {id}"));
        let state = self.props.state();
        match &state {
            Some(State::Current) if self.props.head_known() => {
                ui.label("State: current");
            }
            Some(State::Current) => {
                ui.label("State: current (the head revision could not be checked)");
            }
            Some(other) => {
                ui.colored_label(RED, format!("State: {}", other.label()));
            }
            None => {}
        }
        let editable = matches!(state, Some(State::Current));
        let busy = self.props.busy() || self.other_command_running();
        if let Some(details) = self.props.details().cloned() {
            ui.label(format!(
                "{} read from revision {}.",
                details.kind,
                details.revision.label()
            ));
            // One wrapped row per value: the inspector has a fixed width and values can be long.
            let muted = crate::theme::Palette::current(ui).muted;
            for (label, value) in details.read_only_rows() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(label).color(muted));
                    ui.label(value);
                });
            }
            // Borrowed beside the form (disjoint fields), never copied: the panel is laid out in
            // every frame the window draws.
            let materials = &self.tools.definitions.materials;
            ui.add_enabled_ui(editable && !busy, |ui| {
                ui.label("Editable, in mm:");
                field(ui, "Name", &mut self.props.form.name, 140.0);
                choice_combo(
                    ui,
                    "props-material",
                    "Material",
                    &mut self.props.form.material,
                    materials,
                    false,
                );
                if details.is_plate() {
                    field(ui, "Thickness", &mut self.props.form.thickness, 80.0);
                }
                ui.label("Position (absolute):");
                point_fields(ui, &mut self.props.form.translation);
            });
            if !editable {
                ui.label("Editing is off until the properties are current.");
            }
        } else if matches!(state, Some(State::Unknown { .. })) {
            ui.label("No property value is shown: none could be read.");
        }
        // Whether the staged patch is still what the form produces; preparing it builds the whole
        // argument list, so it is asked once per frame unless a button acted.
        let fresh = self.props.can_commit(&self.source.store);
        let staged = self.props.staged().is_some();
        let (mut preview, mut commit, mut cancel, mut reread) = (false, false, false, false);
        ui.horizontal(|ui| {
            preview = ui
                .add_enabled(editable && !busy, egui::Button::new("Preview"))
                .clicked();
            commit = ui
                .add_enabled(fresh && !busy, egui::Button::new("Commit"))
                .clicked();
            cancel = ui
                .add_enabled(staged && !busy, egui::Button::new("Cancel"))
                .clicked();
            reread = ui
                .add_enabled(!busy, egui::Button::new("Re-read"))
                .clicked();
        });
        if preview {
            match self.props.preview(&self.source.store) {
                Ok(_) => self.props_note = None,
                Err(e) => {
                    self.props.cancel();
                    self.props_note = Some(Err(e.to_string()));
                }
            }
        }
        if commit {
            self.start_property_commit();
        }
        if cancel {
            self.props.cancel();
            self.props_note = Some(Ok("Cancelled; nothing was changed.".into()));
        }
        if reread {
            self.reread();
        }
        if let Some(edit) = self.props.staged() {
            ui.label(format!(
                "Preview: Commit runs `beamcraft` with exactly these {} arguments, one per line (not a shell command line). It changes: {}.",
                edit.args.len(),
                edit.changed.join(", ")
            ));
            ui.add(egui::Label::new(egui::RichText::new(edit.argument_list()).monospace()).wrap());
            let current = if preview || commit || cancel || reread {
                self.props.can_commit(&self.source.store)
            } else {
                fresh
            };
            if !current {
                ui.colored_label(
                    RED,
                    "The parameters or the revision changed: preview again.",
                );
            }
        }
        if self.props.busy() {
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

    fn publish_panel(&mut self, ui: &mut egui::Ui) {
        match self.publisher.target() {
            Err(why) => {
                ui.label(why);
            }
            Ok((published, workspace)) => {
                ui.label(format!(
                    "Workspace {} to published store {}.",
                    workspace.display(),
                    published.display()
                ));
                if let Some(status) = &self.publisher.status {
                    ui.label(status.summary());
                }
                let enabled =
                    self.publisher.phase() == Phase::Idle && !self.other_command_running();
                if ui
                    .add_enabled(enabled, egui::Button::new("Publish…"))
                    .clicked()
                    && let Err(why) = self.publisher.request()
                {
                    self.publisher.note = Some(Err(why.into()));
                }
                if self.publisher.phase() == Phase::Running {
                    ui.label("Publishing…");
                }
            }
        }
        match &self.publisher.note {
            Some(Ok(message)) => {
                ui.label(message);
            }
            Some(Err(message)) => {
                ui.colored_label(RED, message);
            }
            None => {}
        }
    }

    /// The confirm dialog of Publish: nothing runs until Publish is pressed in it.
    fn publish_dialog(&mut self, ctx: &egui::Context) {
        if self.publisher.phase() != Phase::Confirming {
            return;
        }
        let list = self
            .publisher
            .args()
            .map(|args| {
                args.iter()
                    .map(|a| crate::json_string(a))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        let (mut confirm, mut cancel) = (false, false);
        egui::Window::new("Publish the workspace?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(
                    "The draft head becomes the next published revision. The engine refuses it when the published head moved since the workspace was forked or when there is nothing to publish; there is no merge or conflict resolution.",
                );
                ui.label("Runs `beamcraft` with exactly these arguments, one per line:");
                ui.add(egui::Label::new(egui::RichText::new(list).monospace()).wrap());
                ui.horizontal(|ui| {
                    confirm = ui.button("Publish").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        if confirm {
            self.start_publish();
        } else if cancel {
            self.publisher.cancel();
        }
    }

    fn edit_panel(&mut self, ui: &mut egui::Ui) {
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
        let action = self.editor.action();
        ui.add_enabled_ui(selected, |ui| match action {
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
        });
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
        if self.other_command_running() {
            self.tool_note = Some(Err(
                "another command is running; wait for it to finish.".into()
            ));
            return;
        }
        let Some(prepared) = self.tools.begin_commit(&self.source.store) else {
            return;
        };
        // A commit ends the placement: the armed start and the ghost belong to no command.
        self.beam_mode = false;
        self.beam.reset();
        self.ghost_end = None;
        self.renderer
            .set_ghost(&self.state.device, &self.state.queue, None);
        let cli = self.source.cli.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.job = Some(rx);
        self.tool_note = None;
        let stats = Arc::clone(&self.stats);
        let spawned = std::thread::Builder::new()
            .name("beamcraft-command".into())
            .spawn(move || {
                let _ = tx.send(command::commit(&cli, &prepared, &stats));
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
            stats: self.stats.sample(),
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
            self.props.clear();
            match self.perf_ray(ordinal + 1) {
                Some(ray) => self.start_select(ray),
                None => self.selects_finished += 1,
            }
        }
        if request.orbit
            && let Err(e) = self.camera.navigate([0.01, 0.0], [0.0; 2], 0.0)
        {
            self.error = Some(e.to_string());
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

    /// Turns viewport placement of the member on or off (issue #134).
    fn toggle_place(&mut self) {
        self.beam_mode = !self.beam_mode;
        self.beam.reset();
        self.ghost_end = None;
        self.renderer
            .set_ghost(&self.state.device, &self.state.queue, None);
        if self.beam_mode {
            self.tool_note = Some(Ok(
                "Placing: the next two clicks in the view fill Start and End; nothing is written until Commit."
                    .into(),
            ));
        }
    }

    /// One click of the placement: the engine's front-face point on the viewed snapshot, else
    /// the ground plane. The first click arms the start, the second fills the form's endpoints.
    /// The query is bounded by the snapshot caps and runs only while placing.
    fn place_beam(&mut self, ray: crate::edit::Ray) {
        let snapshot = self.loaded.as_ref().map(|l| Arc::clone(&l.snapshot));
        let resolved = match snapshot.map(|s| pick_point_in(&s, &ray)) {
            Some(Ok(resolved)) => resolved
                .map(|(point, _)| point)
                .or_else(|| ground_point(&ray)),
            Some(Err(e)) => {
                self.tool_note = Some(Err(e.to_string()));
                return;
            }
            None => ground_point(&ray),
        };
        let Some(point) = resolved else {
            self.tool_note = Some(Err(
                "the click has no world point: aim at the model or the ground plane".into(),
            ));
            return;
        };
        match self.beam.click(point) {
            None => {
                self.ghost_end = None;
                self.tool_note = Some(Ok(format!(
                    "Start {} armed; the ghost follows the pointer.",
                    PointText(point)
                )));
            }
            Some((start, end)) => {
                self.tools.member.set_endpoints(start, end);
                self.beam_mode = false;
                self.ghost_end = None;
                self.renderer
                    .set_ghost(&self.state.device, &self.state.queue, None);
                self.tool_note = Some(Ok(
                    "Endpoints placed: Preview stages the create-member command; the engine builds the shape at Commit.".into(),
                ));
            }
        }
    }

    /// Rebuilds the ghost preview for the pointer's hover point: the engine's own body mesh for
    /// start -> hover, where the pointer ray meets the plane through the start parallel to the
    /// view. Display only: an axis the engine refuses simply draws no ghost, and Commit still
    /// asks the engine.
    fn update_ghost(&mut self, view: &View, ndc: [f32; 2]) {
        let Some(start) = self.beam.start() else {
            return;
        };
        let origin_mm = self
            .loaded
            .as_ref()
            .map_or([0.0; 3], |l| l.summary.origin_mm);
        let Some(ray) = ray_through(view, ndc, origin_mm) else {
            return;
        };
        // The view plane through the start: every pixel off the horizon maps to a point.
        let n = [
            f64::from(view.back[0]),
            f64::from(view.back[1]),
            f64::from(view.back[2]),
        ];
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let denom = dot(ray.direction, n);
        if denom.abs() < 1e-9 {
            return;
        }
        let t = dot(
            [
                start[0] - ray.origin[0],
                start[1] - ray.origin[1],
                start[2] - ray.origin[2],
            ],
            n,
        ) / denom;
        if !(t.is_finite() && t > 0.0) {
            return;
        }
        let end = [
            ray.origin[0] + t * ray.direction[0],
            ray.origin[1] + t * ray.direction[1],
            ray.origin[2] + t * ray.direction[2],
        ];
        let key = end.map(|v| v.round() as i64);
        if self.ghost_end == Some(key) {
            return;
        }
        self.ghost_end = Some(key);
        let ghost = self.build_ghost(start, end, origin_mm);
        self.renderer
            .set_ghost(&self.state.device, &self.state.queue, ghost.as_ref());
    }

    /// The engine's body mesh for the staged axis, when a rectangle profile is chosen and the
    /// engine accepts the inputs; `None` draws no ghost.
    fn build_ghost(
        &self,
        start: [f64; 3],
        end: [f64; 3],
        origin_mm: [f64; 3],
    ) -> Option<GhostMesh> {
        let rect = self.profile_rect()?;
        // The form's orientation text as the placement quaternion: empty is the identity the
        // commit runs, and a field that does not parse draws no ghost. A well-formed
        // quaternion the engine refuses (a non-normalized one, say) is refused below the same
        // way, so the preview never disagrees with what Commit would build.
        let orientation = crate::tools::orientation_or_identity(&self.tools.member.orientation)?;
        let authored = AuthoredMember {
            id: ElementId::parse("01990000-0000-7000-8000-00000000e347").ok()?,
            kind: match self.tools.member.kind {
                crate::tools::MemberKind::Beam => ModelMemberKind::Beam,
                crate::tools::MemberKind::Column => ModelMemberKind::Column,
            },
            name: "ghost".into(),
            axis_points_mm: [start, end],
            profile: self.tools.member.profile.as_deref()?.parse().ok()?,
            roll_rad: self.tools.member.roll.trim().parse::<f64>().ok(),
            placement: Placement {
                translation_mm: [0.0; 3],
                orientation_xyzw: orientation,
            },
            material: None,
            cut: None,
        };
        let body = beamcraft_engine::member::evaluate_member(
            &authored,
            ProfileInfo::Rectangle {
                width_mm: rect[0],
                height_mm: rect[1],
                declared_area_mm2: None,
            },
        )
        .ok()?;
        Some(GhostMesh::of(
            body.solid.vertices(),
            body.solid.triangles(),
            origin_mm,
        ))
    }

    /// Width and height of the selected rectangle profile, for the ghost.
    fn profile_rect(&self) -> Option<[f64; 2]> {
        let digest = self.tools.member.profile.as_ref()?;
        self.tools
            .definitions
            .profiles
            .iter()
            .find(|c| &c.digest == digest)
            .and_then(|c| c.rect_mm)
    }

    fn tools_panel(&mut self, ui: &mut egui::Ui) {
        ui.label("Dimensions in mm.");
        // Borrowed beside the form (disjoint fields), never copied: the panel is laid out in every
        // frame the window draws.
        let definitions = &self.tools.definitions;
        let placing = self.beam_mode;
        let mut place_clicked = false;
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
                // Optional orientation (issue #134): an empty field keeps the flag absent, so
                // the planar command stays exactly what it always was; the rules are the
                // engine's, the form only forwards text.
                field(ui, "Roll (rad)", &mut form.roll, 80.0);
                field(ui, "Orientation X,Y,Z,W", &mut form.orientation, 140.0);
                place_clicked = ui.selectable_label(placing, "Place in view").clicked();
                if placing {
                    match self.beam.start() {
                        Some(start) => ui.label(format!(
                            "Placing: click the end point in the view (start {}).",
                            PointText(start)
                        )),
                        None => ui.label(
                            "Placing: click the start point in the view. A click lands on a face; in empty space it lands on the ground plane z = 0.",
                        ),
                    };
                }
                if definitions.profiles.is_empty() {
                    ui.label("The store has no profile definition yet (beamcraft define-profile).");
                }
            }
        }
        if place_clicked {
            self.toggle_place();
        }
        let busy = self.tools.busy();
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
            commit = ui.add_enabled(fresh, egui::Button::new("Commit")).clicked();
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

    fn info(&mut self, ui: &mut egui::Ui) {
        ui.label(format!("{:.0} FPS", self.fps));
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
        ui.horizontal(|ui| {
            ui.label("Navigation:");
            if ui.radio(self.preset == Preset::Mouse, "Mouse").clicked() {
                self.preset = Preset::Mouse;
            }
            if ui
                .radio(self.preset == Preset::Trackpad, "Trackpad (macOS)")
                .clicked()
            {
                self.preset = Preset::Trackpad;
            }
        });
        ui.label(match self.preset {
            Preset::Mouse => {
                "Left button: selection only | Middle drag: pan | Ctrl + middle drag: orbit | Wheel: zoom toward the cursor | +/-: zoom | F: fit | R: reload"
            }
            Preset::Trackpad => {
                "Drag: orbit | Shift+drag or secondary drag: pan | wheel, two-finger scroll or pinch: zoom toward the cursor | +/-: zoom | F: fit | R: reload"
            }
        });
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
        self.stats.frames.add(1);
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32();
        self.last = now;
        if dt > 0.0 {
            self.fps = self.fps * 0.9 + 0.1 / dt;
        }
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
        self.poll_props();
        self.poll_publish();
        self.poll_status();
        self.poll_select();
        self.poll_reload();
        self.perf_step(ui.ctx(), frame.info().cpu_usage);
        if !wants_text && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            // Esc closes the publish dialog first; it clears the selection only when none is open.
            if self.publisher.phase() == Phase::Confirming {
                self.publisher.cancel();
            } else {
                self.invalidate_select();
                self.editor.deselect();
                self.props.clear();
                self.set_bench(workbench::Bench::Select);
            }
        }
        // Another process committed to the store: refresh, exactly as the `R` key does.
        let store_changed = self
            .store_watch
            .as_ref()
            .is_some_and(|rx| rx.try_recv().is_ok());
        if reload || store_changed {
            self.reload();
        }
        let fit = fit || std::mem::take(&mut self.pending_fit);
        let pending_zoom = self.pending_zoom.take();
        let zoom_in = zoom_in || pending_zoom == Some(true);
        let zoom_out = zoom_out || pending_zoom == Some(false);
        if fit && let Some(l) = &self.loaded {
            let (target, extent) = l.summary.framing();
            if let Err(e) = self.camera.fit(target, extent, self.aspect()) {
                self.error = Some(e.to_string());
            }
        }
        if self.text {
            // Header, ribbon and status bar first, then the inspector at a fixed width: the 3D
            // view must not shift sideways when a chosen definition name is wider.
            self.header(ui);
            self.ribbon(ui);
            self.status_bar(ui);
            self.inspector(ui);
            let ctx = ui.ctx().clone();
            self.details_window(&ctx);
            self.publish_dialog(&ctx);
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
            let (delta, shift, ctrl, scroll_y, pinch) = ui.input(|i| {
                (
                    i.pointer.delta(),
                    i.modifiers.shift,
                    i.modifiers.ctrl,
                    if hovered {
                        i.smooth_scroll_delta.y
                    } else {
                        0.0
                    },
                    if hovered { i.zoom_delta() } else { 1.0 },
                )
            });
            let (orbit, pan, zoom) = navigation(
                self.preset,
                delta,
                response.dragged_by(egui::PointerButton::Primary),
                response.dragged_by(egui::PointerButton::Secondary),
                response.dragged_by(egui::PointerButton::Middle),
                ctrl,
                shift,
                self.camera.distance(),
                hovered,
                scroll_y,
                pinch,
                zoom_in,
                zoom_out,
                self.invert_zoom,
            );
            let aspect = self.aspect();
            // Zoom toward the cursor: keep the point under the pointer still while zooming. A
            // hostile cursor or an extreme distance falls back to the view centre.
            let pan = if zoom == 0.0 {
                pan
            } else {
                // hover_pos (not interact_pointer_pos): egui fills the latter only while a
                // button is down on this widget, so wheel and pinch zoom need the hover point.
                let ndc = response.hover_pos().and_then(|pos| {
                    ndc_of(
                        [pos.x, pos.y],
                        [response.rect.min.x, response.rect.min.y],
                        [response.rect.width(), response.rect.height()],
                    )
                });
                match ndc.map(|n| self.camera.cursor_zoom_pan(n, aspect, zoom)) {
                    Some(Ok(extra)) => [pan[0] + extra[0], pan[1] + extra[1]],
                    _ => pan,
                }
            };
            let mut picked = None;
            match self
                .camera
                .navigate(orbit, pan, zoom)
                .and_then(|()| self.camera.view(aspect))
            {
                Ok(view) => {
                    // A pale blueprint behind the model in the light theme, a deep one in the dark.
                    let rgb = if ui.ctx().theme() == egui::Theme::Dark {
                        [0x0B, 0x2A, 0x33]
                    } else {
                        [0xE3, 0xF5, 0xFA]
                    };
                    let [r, g, b] = rgb.map(|c: u8| f64::from(c) / 255.0);
                    self.renderer.background = wgpu::Color { r, g, b, a: 1.0 };
                    if let Some(l) = &self.loaded {
                        self.renderer.draw(
                            &self.state.device,
                            &self.state.queue,
                            &l.uploaded,
                            view,
                            &self.stats,
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
                            && let Some(ray) = ray_through(&view, ndc, l.summary.origin_mm)
                        {
                            picked = Some(ray);
                        }
                    }
                    // The ghost preview follows the pointer on the plane through the armed
                    // start, parallel to the view; the committed endpoint comes from the click.
                    if self.beam_mode
                        && let Some(pos) = response.hover_pos()
                        && let Some(ndc) = ndc_of(
                            [pos.x, pos.y],
                            [response.rect.min.x, response.rect.min.y],
                            [response.rect.width(), response.rect.height()],
                        )
                    {
                        self.update_ghost(&view, ndc);
                    }
                }
                Err(e) => self.error = Some(e.to_string()),
            }
            if let Some(ray) = picked {
                if self.beam_mode {
                    self.place_beam(ray);
                } else {
                    self.start_select(ray);
                }
            }
            if self.text {
                self.view_controls(ui, response.rect);
            }
        });
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        App, Source, WATCH_INTERVAL, load_icon, needs_refit, revisions_token, spawn_watch,
        target_size, volume_note,
    };

    #[test]
    fn a_committed_revision_moves_the_watch_token() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("bc-watch-test-{}-{nanos}", std::process::id()));
        let revisions = dir.join("revisions");
        // No revisions directory yet: no signal, and never an error to react to.
        assert_eq!(revisions_token(&dir), None);
        std::fs::create_dir_all(&revisions).unwrap();
        // An empty directory is the zero head: no entries, no newest name.
        assert_eq!(revisions_token(&dir), Some((0, String::new())));
        // A commit is one new record file: the token moves without trusting timestamps.
        std::fs::write(revisions.join("00000000000000000001"), b"x").unwrap();
        assert_eq!(
            revisions_token(&dir),
            Some((1, "00000000000000000001".into()))
        );
        std::fs::write(revisions.join("00000000000000000002"), b"x").unwrap();
        assert_eq!(
            revisions_token(&dir),
            Some((2, "00000000000000000002".into()))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_watch_thread_exits_when_the_owner_drops_the_shutdown() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("bc-watch-exit-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(dir.join("revisions")).unwrap();
        let (_rx, shutdown, handle) = spawn_watch(dir.clone()).unwrap();
        // No filesystem change happens after this point; dropping the shutdown handle must
        // still end the thread.
        drop(shutdown);
        handle.join().expect("the watcher exits on shutdown");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_foreign_commit_wakes_the_watch_thread() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("bc-watch-wake-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(dir.join("revisions")).unwrap();
        let (rx, shutdown, handle) = spawn_watch(dir.clone()).unwrap();
        // The watcher takes its first token at startup, so a commit inside that window is
        // the baseline (missed until the next one, by design). Sleep past one poll, then
        // commit: the signal must arrive.
        std::thread::sleep(WATCH_INTERVAL + std::time::Duration::from_millis(150));
        std::fs::write(dir.join("revisions").join("00000000000000000001"), b"x").unwrap();
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("the watcher signals the committed head");
        drop(shutdown);
        let _ = handle.join();
        std::fs::remove_dir_all(&dir).unwrap();
    }
    use crate::Error;
    use crate::perf::RefreshTiming;
    use crate::render::Gpu;
    use crate::scene::Scene;
    use crate::source::{Refreshed, Reloaded};
    use crate::tools::ToolError;
    use beamcraft_engine::synthetic::plate_scene;
    use beamcraft_engine::{Limits, Representation, ViewPin, ViewSnapshot};
    use beamcraft_model::digest::Sha256;
    use eframe::egui_wgpu;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    /// The window state on the headless adapter, with no window and no CLI behind it: the
    /// refresh it starts fails and is never read, and a test hands it results itself. `None`,
    /// saying so, without a native adapter. The counters are this test's own, so parallel tests
    /// never count into each other (issue 167).
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
        });
        let missing = std::env::temp_dir().join("beamcraft-window-test-no-cli");
        let source = Source {
            store: missing.clone(),
            cli: missing,
            workspace: None,
            published: None,
            samples: None,
            actor: None,
            environment: None,
            author: None,
        };
        let stats = Arc::new(crate::stats::Stats::default());
        Some(App::new(&cc, source, stats).unwrap())
    }

    /// A refresh that read `scene`.
    fn reloaded(scene: Result<Refreshed, Error>) -> Reloaded {
        Reloaded {
            scene,
            definitions: Err(ToolError::local("NOT_FOUND", "no store in this test")),
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
        let token = app.loaded.as_ref().map(|l| l.snapshot.token().clone());
        let swaps = app.stats.reloads_applied.get();
        let uploads = app.stats.uploads.get();
        deliver(&mut app, reloaded(Ok(Refreshed::Unchanged)));
        assert_eq!(
            app.loaded.as_ref().map(|l| l.snapshot.token().clone()),
            token
        );
        // Nothing was built, uploaded or swapped for the snapshot already on screen.
        assert_eq!(app.stats.reloads_applied.get(), swaps);
        assert_eq!(app.stats.uploads.get(), uploads);
        // The scene keeps no display buffer once it is on the device.
        assert!(app.loaded.is_some());
    }

    #[test]
    fn a_refreshed_scene_is_drawn_again_even_when_its_framing_is_unchanged() {
        let Some(mut app) = headless() else { return };
        deliver(&mut app, refreshed(2));
        assert!(app.loaded.is_some());
        let framed = app.framed;
        let swaps = app.stats.reloads_applied.get();
        // The same bounds again, as after an edit inside them: the camera stays where the user
        // left it, and the new scene still replaces the old one.
        deliver(&mut app, refreshed(2));
        assert_eq!(app.framed, framed);
        assert_eq!(
            app.stats.reloads_applied.get(),
            swaps + 1,
            "the new scene must replace the old image"
        );
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

    /// A development build finds the application icon in the source tree; the coded fallback is
    /// a complete icon too, never an empty one.
    #[test]
    fn the_icon_loader_uses_the_app_icon_and_has_a_complete_fallback() {
        let icon = load_icon();
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
        let fallback = super::icon();
        assert_eq!((fallback.width, fallback.height), (32, 32));
        assert_eq!(fallback.rgba.len(), 32 * 32 * 4);
    }
}

#[cfg(test)]
mod navigation_tests {
    use super::{Preset, navigation};
    use crate::camera::Camera;

    const KEYS_NONE: (bool, bool) = (false, false);
    const NO_BUTTON: (bool, bool, bool) = (false, false, false);

    /// One `navigation` call with the flat argument list folded into tuples.
    #[allow(clippy::too_many_arguments)] // test wrapper over the already flat mapping
    fn navigate(
        preset: Preset,
        delta: egui::Vec2,
        buttons: (bool, bool, bool),
        ctrl: bool,
        shift: bool,
        distance: f32,
        hovered: bool,
        scroll_y: f32,
        pinch: f32,
        keys: (bool, bool),
        invert: bool,
    ) -> ([f32; 2], [f32; 2], f32) {
        navigation(
            preset, delta, buttons.0, buttons.1, buttons.2, ctrl, shift, distance, hovered,
            scroll_y, pinch, keys.0, keys.1, invert,
        )
    }

    /// The zoom argument for key-only input at the given toggle state.
    fn key_zoom(zoom_in: bool, zoom_out: bool, invert: bool) -> f32 {
        navigate(
            Preset::Mouse,
            egui::Vec2::ZERO,
            NO_BUTTON,
            false,
            false,
            1000.0,
            false,
            0.0,
            1.0,
            (zoom_in, zoom_out),
            invert,
        )
        .2
    }

    #[test]
    fn mouse_left_drag_never_moves_the_camera() {
        let delta = egui::Vec2::new(10.0, -20.0);
        for shift in [false, true] {
            for ctrl in [false, true] {
                let (orbit, pan, zoom) = navigate(
                    Preset::Mouse,
                    delta,
                    (true, false, false),
                    ctrl,
                    shift,
                    1000.0,
                    false,
                    0.0,
                    1.0,
                    KEYS_NONE,
                    false,
                );
                assert_eq!((orbit, pan, zoom), ([0.0; 2], [0.0; 2], 0.0));
            }
        }
    }

    #[test]
    fn mouse_middle_drag_pans_and_ctrl_middle_drag_orbits() {
        let delta = egui::Vec2::new(10.0, -20.0);
        let (orbit, pan, _) = navigate(
            Preset::Mouse,
            delta,
            (false, false, true),
            false,
            false,
            1000.0,
            false,
            0.0,
            1.0,
            KEYS_NONE,
            false,
        );
        assert_eq!(orbit, [0.0; 2]);
        // The pan factor is distance * 0.0008; 0.0008 is not exact in f32.
        assert!(
            (pan[0] + 8.0).abs() < 1e-4 && (pan[1] + 16.0).abs() < 1e-4,
            "{pan:?}"
        );
        let (orbit, pan, _) = navigate(
            Preset::Mouse,
            delta,
            (false, false, true),
            true,
            false,
            1000.0,
            false,
            0.0,
            1.0,
            KEYS_NONE,
            false,
        );
        assert!(
            (orbit[0] - 0.08).abs() < 1e-6 && (orbit[1] - 0.16).abs() < 1e-6,
            "{orbit:?}"
        );
        assert_eq!(pan, [0.0; 2]);
    }

    #[test]
    fn mouse_scroll_zooms_like_the_trackpad_scroll() {
        let wheel = navigate(
            Preset::Mouse,
            egui::Vec2::ZERO,
            NO_BUTTON,
            false,
            false,
            1000.0,
            true,
            50.0,
            1.0,
            KEYS_NONE,
            false,
        )
        .2;
        let trackpad = navigate(
            Preset::Trackpad,
            egui::Vec2::ZERO,
            NO_BUTTON,
            false,
            false,
            1000.0,
            true,
            50.0,
            1.0,
            KEYS_NONE,
            false,
        )
        .2;
        assert!((wheel - 0.1).abs() < 1e-6, "{wheel}");
        assert_eq!(wheel, trackpad);
    }

    #[test]
    fn shift_turns_the_trackpad_primary_drag_into_a_pan() {
        let delta = egui::Vec2::new(10.0, -20.0);
        let (orbit, pan, _) = navigate(
            Preset::Trackpad,
            delta,
            (true, false, false),
            false,
            false,
            1000.0,
            false,
            0.0,
            1.0,
            KEYS_NONE,
            false,
        );
        assert!(
            (orbit[0] - 0.08).abs() < 1e-6 && (orbit[1] - 0.16).abs() < 1e-6,
            "{orbit:?}"
        );
        assert_eq!(pan, [0.0; 2]);
        // A Shift + primary drag pans exactly like a secondary drag.
        let (_, shifted, _) = navigate(
            Preset::Trackpad,
            delta,
            (true, false, false),
            false,
            true,
            1000.0,
            false,
            0.0,
            1.0,
            KEYS_NONE,
            false,
        );
        let (_, secondary, _) = navigate(
            Preset::Trackpad,
            delta,
            (false, true, false),
            false,
            false,
            1000.0,
            false,
            0.0,
            1.0,
            KEYS_NONE,
            false,
        );
        assert_eq!(shifted, secondary);
    }

    #[test]
    fn the_invert_toggle_flips_only_the_scroll_sign() {
        let scroll = |invert: bool| {
            navigate(
                Preset::Trackpad,
                egui::Vec2::ZERO,
                NO_BUTTON,
                false,
                false,
                1000.0,
                true,
                50.0,
                1.0,
                KEYS_NONE,
                invert,
            )
            .2
        };
        assert!((scroll(false) - 0.1).abs() < 1e-6, "{}", scroll(false));
        assert!((scroll(true) + 0.1).abs() < 1e-6, "{}", scroll(true));
        // Pinch and keys keep their direction under the toggle.
        let pinch = |invert: bool| {
            navigate(
                Preset::Trackpad,
                egui::Vec2::ZERO,
                NO_BUTTON,
                false,
                false,
                1000.0,
                true,
                0.0,
                2.0,
                KEYS_NONE,
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
        for preset in [Preset::Mouse, Preset::Trackpad] {
            let (orbit, pan, zoom) = navigate(
                preset,
                egui::Vec2::ZERO,
                NO_BUTTON,
                false,
                false,
                1000.0,
                false,
                50.0,
                2.0,
                KEYS_NONE,
                false,
            );
            assert_eq!((orbit, pan, zoom), ([0.0; 2], [0.0; 2], 0.0));
        }
    }

    #[test]
    fn navigation_output_stays_inside_the_camera_bounds() {
        let mut camera = Camera::new([0.0; 3], 100.0, 1.0).unwrap();
        let before = camera.distance();
        // A spread pinch (factor 2) halves the distance through the logarithmic path.
        let (orbit, pan, zoom) = navigate(
            Preset::Trackpad,
            egui::Vec2::ZERO,
            NO_BUTTON,
            false,
            false,
            1000.0,
            true,
            0.0,
            2.0,
            KEYS_NONE,
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
            let (orbit, pan, zoom) = navigate(
                Preset::Trackpad,
                egui::Vec2::ZERO,
                NO_BUTTON,
                false,
                false,
                1000.0,
                true,
                0.0,
                hostile,
                KEYS_NONE,
                false,
            );
            camera.navigate(orbit, pan, zoom).unwrap();
            assert!(camera.distance().is_finite(), "pinch {hostile}");
        }
        // The same holds for a mouse pan with an absurd pointer delta: the bounded navigate
        // rejects the frame outright.
        let (orbit, pan, zoom) = navigate(
            Preset::Mouse,
            egui::Vec2::new(-1.0e9, 1.0e9),
            (false, false, true),
            false,
            false,
            2e7,
            false,
            0.0,
            1.0,
            KEYS_NONE,
            false,
        );
        assert!(camera.navigate(orbit, pan, zoom).is_err());
        assert!(camera.distance().is_finite(), "mouse pan");
    }
}
