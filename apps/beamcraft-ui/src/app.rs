// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The eframe window: a side panel with the revision, counts, units and adapter, and the viewport
//! navigated through one of two input presets: `Mouse` (the default: wheel-button drag pans,
//! Ctrl + wheel-button drag orbits, the left button stays free for selection) or `Trackpad`
//! (primary drag orbits, Shift + primary drag pans, pinch zooms). In both the wheel zooms toward
//! the cursor, F fits and R reloads.

mod annotation_panel;
mod creation_panel;
mod grid_panel;
mod switches;
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
use beamcraft_engine::RenderMode;
use beamcraft_engine::ViewSnapshot;
use beamcraft_engine::element::ElementId;
use beamcraft_engine::views::{NamedView, ViewBook, WorkArea};
use beamcraft_model::command::Placement;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};
use steel::engine::member::{AuthoredMember, ProfileInfo};
use steel::model::command::MemberKind as ModelMemberKind;

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
    /// The user's configuration directory (`--config-dir`); `None` is the platform default. The
    /// render-mode shortcuts are read from it, nothing else.
    pub config_dir: Option<PathBuf>,
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
    pub(crate) appearance: Option<crate::appearance::Appearance>,
    pub(crate) from_library: bool,
    pub(crate) return_to_library: bool,
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
    plate_mode: bool,
    plate_draft: crate::creation::ContourDraft,
    plate_hover: Option<[f64; 3]>,
    snap_hover: Option<beamcraft_engine::query::pick::SnapKind>,
    viewport_height: f32,
    distance_input: String,
    distance_aim: Option<[f64; 3]>,
    plate_input: [String; 2],
    plane_form: crate::work_plane::Form,
    plane_pick: bool,
    /// The millimetre-quantized hover end the current ghost was built for: the buffers rebuild
    /// only when the preview actually moves.
    ghost_end: Option<[i64; 3]>,
    beam_hover: Option<[f64; 3]>,
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
    /// The D05 sheet editor: hand placement of a drawing document's views.
    sheet_editor: crate::sheet::SheetEditor,
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
    /// The command category shown on the ribbon.
    ribbon_tab: workbench::RibbonTab,
    /// Whether the workspace panel is expanded along the left side of the window.
    menu_open: bool,
    grid_form: crate::grid::GridForm,
    grid_line: crate::grid::DrawLineForm,
    grid_drawn: bool,
    grid_scene: crate::grid::GridScene,
    grid_job: Option<Receiver<Result<crate::grid::CreatedGrid, ToolError>>>,
    grid_note: Option<Result<String, String>>,
    grids_visible: bool,
    grid_edit: Option<grid_panel::GridEdit>,
    grid_refresh_edit: bool,
    /// D06 annotations: placement tool, listed rows and their jobs.
    annotation: annotation_panel::Panel,
    /// The bottom bar's selection switches and snap panel (V11): picking-only state.
    selection: crate::selection::Selection,
    /// Whether the Details window is open.
    menu_section: workbench::MenuSection,
    pub(crate) return_to_create: bool,
    command_search: bool,
    command_focus: bool,
    command_query: String,
    browser_search: String,
    /// Whether the Details window is open.
    details: bool,
    /// A fit requested by the ribbon or command search, applied in the next frame.
    pending_fit: bool,
    logo: Option<egui::TextureHandle>,
    logo_tried: bool,
    /// The active render mode (V16); [`beamcraft_engine::RenderMode::Shaded`] is the default and
    /// switching never reloads the scene.
    mode: beamcraft_engine::RenderMode,
    /// The render-mode shortcuts: the user's file or the defaults.
    shortcuts: crate::shortcuts::Shortcuts,
    /// Why the shortcuts file was not used, when it was not.
    shortcuts_note: Option<String>,
    /// The selected-only draw and the (selected id, scene origin) it was built for, so the one
    /// engine query runs on a selection or reload change, not per frame.
    selected_only: ((String, [f64; 3]), Option<crate::scene::SelectedDraw>),
    /// Actual world boundary segments, cached by selected identity and snapshot token.
    selection_edges: ((String, String), Vec<[[f64; 3]; 2]>),
    /// The uncommitted cut-plane form (V15); applying it fills `cut`.
    cut_form: crate::cut_plane::Form,
    /// The window's applied display cut; `None` shows the whole model.
    cut: Option<beamcraft_engine::view_clipping::Clipping>,
    /// The named views of this model (V12) with their session sidecar state.
    views: ViewBook,
    views_note: Option<String>,
    /// The work area the current view carries; a view property, never model state.
    work_area: Option<WorkArea>,
    /// The saved-views window and the extra view windows.
    views_panel: crate::views::Panel,
    view_windows: Vec<crate::views::ViewWindow>,
    /// The saved representation sets of the user (V17): the class preset plus
    /// `colour-sets.json`.
    colour_sets: crate::colour_sets::Sets,
    /// Why the colour-sets file was not used, when it was not.
    colour_sets_note: Option<String>,
    /// The set the colour combo shows; applying it evaluates it and fills `colour`.
    colour_pick: String,
    /// The applied colour overlay and the set it was built from; `None` colours nothing (V17).
    colour: Option<(String, Arc<crate::colour_sets::Overlay>)>,
    /// A set evaluation in flight.
    colour_job: Option<ColourJob>,
    /// The last colour outcome, shown in the colour panel.
    colour_note: Option<Result<String, String>>,
    /// The overlay the running refresh builds its scene with (V17): if the user changes or
    /// clears the set while the refresh runs, the delivered scene is recoloured once, so stale
    /// colours can never return or outlive their set.
    reload_overlay: Option<Arc<crate::colour_sets::Overlay>>,
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

/// A set evaluation in flight (V17): the set it colours, its receiver and the flag that gives
/// it up when the overlay is cleared while the queries still run.
struct ColourJob {
    set: String,
    rx: Receiver<Result<crate::colour_sets::Overlay, String>>,
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
    // Drag the visible model with the pointer; the orbit camera moves the other way.
    let orbit = [-pointer_delta.x * 0.008, pointer_delta.y * 0.008];
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

fn definition_label(label: &str) -> &str {
    label
        .rsplit_once(" (")
        .filter(|(_, tail)| {
            tail.trim_end_matches(')')
                .chars()
                .all(|c| c.is_ascii_hexdigit())
        })
        .map_or(label, |(name, _)| name)
}

fn unique_suffix<'a>(id: &'a str, others: &[&str]) -> &'a str {
    let mut length = id.len().min(8);
    loop {
        let suffix = id.get(id.len().saturating_sub(length)..).unwrap_or(id);
        if length >= id.len()
            || !others
                .iter()
                .any(|other| *other != id && other.ends_with(suffix))
        {
            return suffix;
        }
        length += 1;
    }
}

fn definition_choice_label(choice: &Choice, choices: &[Choice]) -> String {
    let label = definition_label(&choice.label);
    let duplicates = choices
        .iter()
        .filter(|c| definition_label(&c.label) == label)
        .map(|c| c.digest.as_str())
        .collect::<Vec<_>>();
    if duplicates.iter().any(|id| *id != choice.digest) {
        format!("{label} · {}", unique_suffix(&choice.digest, &duplicates))
    } else {
        label.to_owned()
    }
}

fn level_choice_label(id: &str, levels: &[(String, String, f64)]) -> String {
    let Some((_, name, elevation)) = levels.iter().find(|(known, _, _)| known == id) else {
        return "Level unavailable".into();
    };
    let duplicates = levels
        .iter()
        .filter(|(_, n, e)| n == name && e == elevation)
        .map(|(known, _, _)| known.as_str())
        .collect::<Vec<_>>();
    let label = format!("{name} · {elevation} mm");
    if duplicates.iter().any(|known| *known != id) {
        format!("{label} · {}", unique_suffix(id, &duplicates))
    } else {
        label
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
        .map(|digest| {
            choices.iter().find(|c| &c.digest == digest).map_or_else(
                || "Definition unavailable".to_owned(),
                |c| definition_choice_label(c, choices),
            )
        })
        .unwrap_or_else(|| if required { "choose…" } else { "none" }.to_owned());
    // The label sits left of the list (as with the text fields): drawn below it, it read as
    // the label of the next drop-down.
    ui.horizontal(|ui| {
        ui.add_sized(
            egui::vec2(70.0, 28.0),
            egui::Label::new(egui::RichText::new(label).size(11.0)),
        );
        egui::ComboBox::from_id_salt(id)
            .width(ui.available_width().max(80.0))
            .selected_text(selected)
            .show_ui(ui, |ui| {
                if !required {
                    ui.selectable_value(current, None, "none");
                }
                for choice in choices {
                    ui.selectable_value(
                        current,
                        Some(choice.digest.clone()),
                        definition_choice_label(choice, choices),
                    );
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

/// Draws the selected body's evaluated boundary instead of inventing corners from its bounds.
fn draw_selection(
    painter: &egui::Painter,
    rect: egui::Rect,
    view: &View,
    origin: [f64; 3],
    edges: &[[[f64; 3]; 2]],
) {
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
    let stroke = egui::Stroke::new(2.0, crate::theme::BRAND);
    for [a, b] in edges {
        if let (Some(p), Some(q)) = (project(*a), project(*b)) {
            painter.line_segment([p, q], stroke);
        }
    }
}

/// Viewport clicks do not capture keyboard focus from construction input.
fn viewport_sense() -> egui::Sense {
    egui::Sense::CLICK | egui::Sense::DRAG
}

fn field(ui: &mut egui::Ui, label: &str, text: &mut String, width: f32) {
    ui.horizontal(|ui| {
        ui.add_sized(
            egui::vec2(70.0, 28.0),
            egui::Label::new(egui::RichText::new(label).size(11.0)),
        );
        ui.add(
            egui::TextEdit::singleline(text)
                .desired_width(width.min(ui.available_width()))
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
        let appearance = crate::appearance::Appearance::new(None);
        let mut app = Self::open(
            &cc.egui_ctx,
            cc.wgpu_render_state.as_ref(),
            source,
            perf,
            stats,
        )?;
        app.appearance = Some(appearance);
        Ok(app)
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
        let (shortcuts, shortcuts_note) =
            crate::shortcuts::Shortcuts::load(source.config_dir.as_deref());
        let (colour_sets, colour_sets_note) =
            crate::colour_sets::Sets::load(source.config_dir.as_deref());
        let colour_pick = colour_sets
            .list()
            .first()
            .map(|set| set.name.clone())
            .unwrap_or_default();
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
        // The named-view sidecar is display state: a missing file is an empty book, and an
        // unreadable one only notes, never blocks opening the model.
        let (views, views_note) = match crate::views::load(&source.store) {
            Ok(book) => (book, None),
            Err(note) => (beamcraft_engine::views::ViewBook::new(), Some(note)),
        };
        let mut app = Self {
            appearance: None,
            from_library: false,
            return_to_library: false,
            ribbon_tab: workbench::RibbonTab::Steel,
            menu_open: false,
            grid_form: crate::grid::GridForm::default(),
            grid_scene: crate::grid::GridScene::default(),
            grid_job: None,
            grid_note: None,
            grids_visible: true,
            annotation: annotation_panel::Panel::empty(),
            menu_section: workbench::MenuSection::default(),
            return_to_create: false,
            command_search: false,
            command_focus: false,
            command_query: String::new(),
            browser_search: String::new(),
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
            plate_mode: false,
            plate_draft: crate::creation::ContourDraft::default(),
            plate_hover: None,
            snap_hover: None,
            viewport_height: 800.0,
            distance_input: String::new(),
            distance_aim: None,
            plate_input: ["0".into(), "0".into()],
            plane_form: crate::work_plane::Form::default(),
            plane_pick: false,
            ghost_end: None,
            beam_hover: None,
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
            sheet_editor: crate::sheet::SheetEditor::default(),
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
            grid_line: crate::grid::DrawLineForm::default(),
            grid_drawn: false,
            grid_edit: None,
            grid_refresh_edit: false,
            selection: crate::selection::Selection::default(),
            details: false,
            pending_fit: false,
            logo: None,
            logo_tried: false,
            mode: beamcraft_engine::RenderMode::default(),
            cut_form: crate::cut_plane::Form::default(),
            cut: None,
            shortcuts,
            shortcuts_note,
            selected_only: ((String::new(), [0.0; 3]), None),
            selection_edges: Default::default(),
            views,
            views_note,
            work_area: None,
            views_panel: crate::views::Panel::default(),
            view_windows: Vec::new(),
            colour_sets,
            colour_sets_note,
            colour_pick,
            colour: None,
            colour_job: None,
            colour_note: None,
            reload_overlay: None,
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
        // The overlay is shared with the worker so the new scene is coloured as it is built;
        // the window keeps the same Arc to recognise a stale delivery when the set changes.
        let overlay = self.colour.as_ref().map(|(_, overlay)| Arc::clone(overlay));
        let delivered = overlay.clone();
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
                let _ = tx.send(source::reload(
                    &cli,
                    &store,
                    &known,
                    overlay.as_deref(),
                    &stats,
                ));
            });
        match spawned {
            Ok(_) => {
                self.reload_overlay = delivered;
                self.reload_job = Some(rx);
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
                self.reload_overlay = None;
                self.error = Some("the reload thread ended without a result".into());
                return;
            }
        };
        self.reload_job = None;
        // What the delivered scene was coloured with, to compare against the set the panel
        // shows now (a change during the refresh is applied to the delivery, not lost).
        let delivered = self.reload_overlay.take();
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
        let grids = reloaded.grids;
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
                self.apply_grids(grids);
                if let Some((target, extent)) =
                    self.loaded.as_ref().map(|l| self.grid_framing(&l.summary))
                {
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
                let next_grids = grids.unwrap_or_else(|error| {
                    self.grid_note = Some(Err(error.to_string()));
                    crate::grid::GridScene::default()
                });
                let (target, extent) = if self.grids_visible {
                    next_grids.framing(&loaded.summary)
                } else {
                    loaded.summary.framing()
                };
                let aspect = self.aspect();
                // A scene the camera cannot frame keeps the previous scene and shows why. An
                // unchanged framing keeps the user's camera.
                if needs_refit(self.framed, target, extent, aspect) {
                    match self.camera.fit(target, extent, aspect) {
                        Ok(()) => {
                            self.framed = Some((target, extent, aspect));
                            self.grid_scene = next_grids;
                            self.show(loaded);
                        }
                        Err(e) => self.error = Some(e.to_string()),
                    }
                } else {
                    self.grid_scene = next_grids;
                    self.show(loaded);
                }
            }
            // A store with no revision yet is a normal state, not an error: the definitions
            // listing treats the same NOT_FOUND the same way.
            Err(e) if source::is_no_revision(&e) => {
                self.grid_scene = crate::grid::GridScene::default();
                self.loaded = None;
                self.framed = None;
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
        // The delivered scene was built with the overlay captured at spawn (V17): when the user
        // changed or cleared the set while the refresh ran, the delivery is recoloured once, so
        // stale colours can neither return after a Hide nor outlive their set behind a legend.
        if self.loaded.is_some() {
            let current = self.colour.as_ref().map(|(_, overlay)| Arc::clone(overlay));
            let stale = match (&delivered, &current) {
                (Some(built), Some(active)) => !Arc::ptr_eq(built, active),
                (Some(_), None) | (None, Some(_)) => true,
                (None, None) => false,
            };
            if stale {
                self.recolour();
            }
        }
        self.sync_grid_selection();
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
        // A new revision may have moved or replaced the selected element: the selected-only
        // draw is rebuilt against this scene.
        self.selected_only = ((String::new(), [0.0; 3]), None);
        self.selection_edges = Default::default();
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

    /// The saved-views window (V12): one place to apply, capture and edit views.
    fn views_window(&mut self, ctx: &egui::Context) {
        if !self.views_panel.window_open {
            return;
        }
        let mut open = true;
        let mut shown: Option<Result<Option<crate::views::Action>, String>> = None;
        egui::Window::new("Saved views")
            .open(&mut open)
            .default_width(430.0)
            .show(ctx, |ui| {
                // Why the last save, removal or restore failed, kept visible until the next
                // action succeeds.
                if let Some(note) = &self.views_note {
                    ui.colored_label(RED, note);
                }
                shown =
                    Some(
                        self.views_panel
                            .show(ui, &self.views, &self.camera, &self.grid_scene),
                    );
            });
        self.views_panel.window_open &= open;
        match shown {
            // The action runs after the window closes its borrow of the panel state.
            Some(Ok(Some(action))) => self.view_action(action),
            Some(Err(note)) => self.views_note = Some(note),
            _ => {}
        }
    }

    /// Runs one explicit saved-views action; field edits never reach here.
    fn view_action(&mut self, action: crate::views::Action) {
        match action {
            crate::views::Action::Apply(name) => {
                if let Some(view) = self.views.get(&name).cloned() {
                    if let Err(e) = self.camera.set_pose(&view.pose) {
                        self.error = Some(e.to_string());
                    }
                    self.work_area = view.work_area;
                    self.views_panel.set_work_area(self.work_area.as_ref());
                    self.views_note = None;
                }
            }
            crate::views::Action::Delete(name) => {
                self.views.remove(&name);
                self.persist_views();
            }
            crate::views::Action::SaveCurrent => {
                let name = self.views_panel.typed_name().to_owned();
                match NamedView::new(&name, None, self.camera.pose(), self.work_area) {
                    Ok(view) => self.save_named_view(view),
                    Err(e) => self.views_note = Some(e.to_string()),
                }
            }
            crate::views::Action::SavePlane(spec) => {
                let name = self.views_panel.typed_name().to_owned();
                let distance = f64::from(self.camera.distance());
                let resolved = beamcraft_engine::views::resolve_plane(
                    &spec,
                    &self.grid_scene.query_grids,
                    self.frame_centre(),
                    distance,
                );
                match resolved {
                    Ok(pose) => match NamedView::new(&name, Some(spec), pose, self.work_area) {
                        Ok(view) => self.save_named_view(view),
                        Err(e) => self.views_note = Some(e.to_string()),
                    },
                    Err(e) => self.views_note = Some(e.to_string()),
                }
            }
            crate::views::Action::SetWorkArea(area) => {
                self.work_area = Some(area);
                self.views_panel.set_work_area(self.work_area.as_ref());
            }
            crate::views::Action::ClearWorkArea => {
                self.work_area = None;
                self.views_panel.set_work_area(None);
            }
            crate::views::Action::ToggleProjection => {
                self.camera.set_projection(!self.camera.is_orthographic());
            }
        }
    }

    /// Saves a named view into the book and its sidecar.
    fn save_named_view(&mut self, view: NamedView) {
        if let Err(e) = self.views.save(view) {
            self.views_note = Some(e.to_string());
            return;
        }
        self.persist_views();
        self.views_note = None;
    }

    /// Persists the book, reporting a failure as a note.
    fn persist_views(&mut self) {
        if let Err(e) = crate::views::persist(&self.source.store, &self.views) {
            self.views_note = Some(e);
        }
    }

    /// The centre of the loaded scene bounds in model millimetres; the frame plane views
    /// centre on. Without a loaded scene it is the origin.
    fn frame_centre(&self) -> [f64; 3] {
        match &self.loaded {
            Some(l) => {
                let axis = |i: usize| {
                    l.summary.origin_mm.get(i).copied().unwrap_or(0.0)
                        + f64::from(
                            (l.summary.min.get(i).copied().unwrap_or(0.0)
                                + l.summary.max.get(i).copied().unwrap_or(0.0))
                                / 2.0,
                        )
                };
                [axis(0), axis(1), axis(2)]
            }
            None => [0.0; 3],
        }
    }

    /// Opens another view window on the current view, up to the cap.
    fn open_view_window(&mut self) {
        if self.view_windows.len() < crate::views::MAX_WINDOWS {
            let camera = self.camera;
            let index = self.view_windows.len();
            self.view_windows
                .push(crate::views::ViewWindow::new(camera, index));
        }
    }

    /// Draws the extra view windows: the loaded scene through their own cameras. They navigate
    /// and zoom like the main viewport; selection and tools stay in the main viewport.
    fn draw_view_windows(&mut self, ctx: &egui::Context) {
        self.view_windows.retain(|w| w.open);
        for window in &mut self.view_windows {
            let mut open = true;
            let title = window.title().to_owned();
            egui::Window::new(title)
                .open(&mut open)
                .default_width(520.0)
                .default_height(390.0)
                .show(ctx, |ui| {
                    let size = ui.available_size();
                    let scale = ui.ctx().pixels_per_point();
                    let [width, height] = target_size([size.x, size.y], scale);
                    let samples = self.source.samples;
                    let Ok(aspect) =
                        window.resize(&self.state, width, height, samples, &self.stats)
                    else {
                        return;
                    };
                    let Some(loaded) = &self.loaded else {
                        return;
                    };
                    // The selected-only mode belongs to the main viewport's inspector; other
                    // windows always draw the whole scene.
                    let mode = if self.mode == RenderMode::SelectedOnly {
                        RenderMode::Shaded
                    } else {
                        self.mode
                    };
                    let Ok(view) = window.camera.view(aspect) else {
                        return;
                    };
                    if let (Some(renderer), Some(texture)) = (window.renderer(), window.texture()) {
                        // A secondary window shows the model as the main viewport does, so the
                        // main window's cut applies to it too (the cut record stays V15's).
                        let clip = match &self.cut {
                            Some(cut) => {
                                crate::cut_plane::ClipGpu::of(cut, loaded.summary.origin_mm)
                            }
                            None => crate::cut_plane::ClipGpu::none(),
                        };
                        renderer.draw(
                            &self.state.device,
                            &self.state.queue,
                            &loaded.uploaded,
                            view,
                            &self.stats,
                            mode,
                            &clip,
                        );
                        let response =
                            ui.add(egui::Image::new((texture, size)).sense(viewport_sense()));
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
                            window.camera.distance(),
                            hovered,
                            scroll_y,
                            pinch,
                            false,
                            false,
                            self.invert_zoom,
                        );
                        if let Err(e) = window.camera.navigate(orbit, pan, zoom) {
                            self.error = Some(e.to_string());
                        }
                    }
                });
            // The close button wrote through `open`: a closed window is gone next frame and
            // its slot is free again.
            window.open &= open;
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
        self.grid_edit = None;
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
        let selection = self.selection.filter();
        let spawned = std::thread::Builder::new()
            .name("beamcraft-pick".into())
            .spawn(move || {
                let _ = tx.send(command::select(
                    &cli, &store, &snapshot, &ray, &cancel, &stats, selection,
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

    /// Delete the current grid or part using the same engine command as the CLI.
    fn delete_selection(&mut self) {
        if self.other_command_running() {
            return;
        }
        if let Some(edit) = &self.grid_edit {
            self.editor.select_id(edit.id.clone());
        }
        if self.editor.selected().is_some() {
            self.start_edit(Some(EditKind::Delete));
        }
    }

    fn selection_shortcuts(&mut self, ctx: &egui::Context) {
        if !ctx.egui_wants_keyboard_input()
            && !self.menu_open
            && self.publisher.phase() != Phase::Confirming
            && ctx.input(|input| input.key_pressed(egui::Key::Delete))
        {
            self.delete_selection();
        }
    }

    /// The render-mode shortcuts (V16): one key per mode, the user's bindings. Ignored while a
    /// text field has the keyboard. Switching a mode changes only the drawing: the loaded
    /// snapshot, its token and the camera stay as they are.
    fn render_mode_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let pressed = ctx.input(|input| self.shortcuts.pressed(input));
        if let Some(mode) = pressed {
            self.mode = mode;
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
                self.edit_note = Some(Ok(format!("{}.", done.kind.done())));
                match done.kind {
                    EditKind::Delete => {
                        self.editor.deselect();
                        self.props.clear();
                        self.grid_edit = None;
                        self.grid_form = crate::grid::GridForm::default();
                        self.set_bench(workbench::Bench::Select);
                    }
                    // The element may have moved, or the undo may have removed it: ask again.
                    _ => self.refresh_selection(),
                }
                self.refresh_status();
            }
            Err(e) => {
                if self.grid_edit.is_some() {
                    self.grid_note = Some(Err(e.to_string()));
                    self.editor.deselect();
                }
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
        self.other_command_running_pre_colour()
            || self.colour_job.is_some()
            || self.annotation.list_job.is_some()
            || self.annotation.commit_job.is_some()
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
                let _ = done;
                self.props_note = Some(Ok("Properties updated.".into()));
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

    /// Runs the confirmed `publish-workspace` on a worker.
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
        let Some(_id) = self.props.selected().map(str::to_owned) else {
            ui.label("Select a part or grid to see its properties.");
            return;
        };
        let palette = crate::theme::Palette::current(ui);
        let state = self.props.state();
        match &state {
            Some(State::Current) if self.props.head_known() => {}
            Some(State::Current) => {
                ui.label("Could not check for model changes. Reload before applying edits.");
            }
            Some(other) => {
                ui.colored_label(
                    RED,
                    match other {
                        State::Stale { .. } => {
                            "The model changed. Reload properties before editing."
                        }
                        _ => "Properties could not be loaded. Try Reload.",
                    },
                );
            }
            None => {}
        }
        let editable = self.props.head_known() && matches!(state, Some(State::Current));
        let busy = self.props.busy() || self.other_command_running();
        if let Some(details) = self.props.details().cloned() {
            if let Some((_, count)) = &details.shared_definition {
                ui.label(format!("Shape and material edits affect all {count} shared parts. Name, position and level affect this part."));
            }
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(details.kind.to_uppercase())
                    .size(10.0)
                    .color(palette.muted),
            );
            ui.label(crate::theme::bold(&details.name, 18.0));
            let subtitle = [
                details.profile_name.as_deref(),
                details.material_name.as_deref(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            if !subtitle.is_empty() {
                ui.label(
                    egui::RichText::new(subtitle)
                        .size(11.0)
                        .color(palette.muted),
                );
            }
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(12.0);
            // Borrowed beside the form (disjoint fields), never copied: the panel is laid out in
            // every frame the window draws.
            let materials = &self.tools.definitions.materials;
            ui.add_enabled_ui(editable && !busy, |ui| {
                ui.label(egui::RichText::new("General").size(12.0));
                field(ui, "Name", &mut self.props.form.name, 170.0);
                field(ui, "Class", &mut self.props.form.class, 80.0);
                if !details.is_plate() {
                    // A member's profile is chosen from the model's and the project environment's
                    // profiles; the engine rebuilds the body and the mass (`update-element`).
                    choice_combo(
                        ui,
                        "props-profile",
                        "Profile",
                        &mut self.props.form.profile,
                        &self.tools.definitions.profiles,
                        false,
                    );
                }
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
                let level_label = self.props.form.level.as_ref().map_or_else(
                    || "Not assigned".into(),
                    |id| level_choice_label(id, &details.levels),
                );
                ui.label("Level");
                egui::ComboBox::from_id_salt("props-level")
                    .selected_text(level_label)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.props.form.level, None, "Not assigned");
                        for (id, _, _) in &details.levels {
                            ui.selectable_value(
                                &mut self.props.form.level,
                                Some(id.clone()),
                                level_choice_label(id, &details.levels),
                            );
                        }
                    });
                ui.add_space(18.0);
                ui.label(egui::RichText::new("Position / mm").size(12.0));
                ui.add_space(8.0);
                for (axis, value) in ["X", "Y", "Z"]
                    .into_iter()
                    .zip(&mut self.props.form.translation)
                {
                    field(ui, axis, value, 170.0);
                }
            });
            egui::CollapsingHeader::new("Quantities")
                .default_open(true)
                .show(ui, |ui| {
                    let quantities = &details.quantities;
                    for (label, number, unit, status, stored_freshness) in [
                        (
                            "Volume",
                            quantities.volume_mm3,
                            "mm³",
                            quantities.volume_status.as_deref(),
                            quantities.stored_volume_freshness.as_deref(),
                        ),
                        (
                            "Mass",
                            quantities.mass_kg,
                            "kg",
                            quantities.mass_status.as_deref(),
                            quantities.stored_mass_freshness.as_deref(),
                        ),
                    ] {
                        let mut value = match (number, status) {
                            (Some(value), Some("current")) => format!("{value} {unit}"),
                            (Some(value), Some("stale")) => format!("{value} {unit} · out of date"),
                            _ => "Not available".into(),
                        };
                        if stored_freshness == Some("stale") {
                            value.push_str(" · stored value out of date");
                        }
                        ui.horizontal(|ui| {
                            ui.label(label);
                            ui.label(value);
                        });
                    }
                });
            if !editable {
                ui.label("Editing is off until the properties are current.");
            }
        } else if matches!(state, Some(State::Unknown { .. })) {
            ui.label("No property value is shown: none could be read.");
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

    fn property_actions(&mut self, ui: &mut egui::Ui) {
        let editable =
            self.props.head_known() && matches!(self.props.state(), Some(State::Current));
        let busy = self.props.busy() || self.other_command_running();
        let mut apply = false;
        let mut reset = false;
        ui.columns(2, |columns| {
            for (column, (kind, label, pressed)) in columns.iter_mut().zip([
                (crate::theme::Kind::Primary, "Apply", &mut apply),
                (crate::theme::Kind::Secondary, "Reset", &mut reset),
            ]) {
                *pressed = column
                    .with_layout(
                        egui::Layout::top_down_justified(egui::Align::Center),
                        |ui| crate::theme::button(ui, kind, label, editable && !busy).clicked(),
                    )
                    .inner;
            }
        });
        if apply {
            match self.props.preview(&self.source.store) {
                Ok(_) => self.start_property_commit(),
                Err(error) => {
                    self.props.cancel();
                    self.props_note = Some(Err(error.to_string()));
                }
            }
        }
        if reset && let Some(details) = self.props.details().cloned() {
            let id = details.id.clone();
            self.props.load(&id, Ok(details));
            self.props_note = None;
        }
    }

    fn publish_panel(&mut self, ui: &mut egui::Ui) {
        match self.publisher.target() {
            Err(why) => {
                ui.label(why);
            }
            Ok(_) => {
                if let Some(status) = &self.publisher.status {
                    ui.label(if status.unsent_revisions == 0 {
                        "All changes are published."
                    } else {
                        "This workspace has unpublished changes."
                    });
                    if !status.base_is_current {
                        ui.colored_label(
                            RED,
                            "The shared model changed. Review it before publishing.",
                        );
                    }
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
        let (mut confirm, mut cancel) = (false, false);
        egui::Window::new("Publish the workspace?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(
                    "Publish your saved workspace changes to the shared model? If the shared model has changed, publishing will stop so your work stays safe.",
                );
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

    /// The V15 cut plane inspector: the form, the applied state, nothing model-touching.
    fn cut_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(
            "The cut plane hides part of the model in this window only. The model, its \
             revisions and every measurement stay unchanged.",
        );
        let mut note = None;
        match self.cut_form.show(ui) {
            Ok(Some(crate::cut_plane::Action::Apply(cut))) => {
                let planes = cut.effective().map_or(0, |p| p.len());
                note = Some(Ok(format!(
                    "Cut applied: {} user plane(s), work-area box {}, {planes} effective \
                     half-space(s).",
                    cut.planes.len(),
                    if cut.area.is_some() { "on" } else { "off" },
                )));
                self.cut = Some(cut);
            }
            Ok(Some(crate::cut_plane::Action::Clear)) => {
                note = Some(Ok("Cut removed: the whole model is visible.".into()));
                self.cut = None;
            }
            Err(error) => note = Some(Err(error.to_string())),
            Ok(None) => {}
        }
        match &self.cut {
            Some(cut) => ui.label(format!(
                "Applied: {} user plane(s), work-area box {}.",
                cut.planes.len(),
                if cut.area.is_some() { "on" } else { "off" },
            )),
            None => ui.label("No cut: the whole model is visible."),
        };
        if let Some(note) = note {
            match note {
                Ok(message) => {
                    ui.label(message);
                }
                Err(message) => {
                    ui.colored_label(crate::theme::Palette::current(ui).error, message);
                }
            }
        }
    }

    /// The V17 colour inspector: pick a saved representation set, apply or clear the overlay.
    /// Everything here is display state: the overlay is never written to the model, and a set
    /// lives in the user's colour-sets file, not in the store.
    fn colour_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(
            "Colour the model by a saved representation set. Colours are display only: the \
             model, its revisions and every measurement stay unchanged, and the set lives in \
             your colour-sets file, never in the model.",
        );
        let palette = crate::theme::Palette::current(ui);
        if let Some(note) = &self.colour_sets_note {
            ui.colored_label(palette.muted, note);
        }
        let busy = self.colour_job.is_some();
        let applied = self
            .colour
            .as_ref()
            .map(|(name, overlay)| (name.clone(), overlay.len()));
        let pick = self.colour_pick.clone();
        ui.add_enabled_ui(!busy, |ui| {
            egui::ComboBox::from_id_salt("colour-set")
                .selected_text(if pick.is_empty() { "No set" } else { &pick })
                .show_ui(ui, |ui| {
                    for set in self.colour_sets.list() {
                        ui.selectable_value(&mut self.colour_pick, set.name.clone(), &set.name);
                    }
                });
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!pick.is_empty(), egui::Button::new("Show colours"))
                    .clicked()
                {
                    self.apply_colour(&pick);
                }
                if ui
                    .add_enabled(applied.is_some(), egui::Button::new("Hide colours"))
                    .clicked()
                {
                    self.clear_colour();
                }
            });
        });
        if busy {
            ui.label("Evaluating the set…");
        }
        // The legend of the applied set: one swatch per rule, in rule order.
        if let Some((name, coloured)) = &applied {
            ui.separator();
            match self.colour_sets.find(name) {
                Some(set) => {
                    ui.label(format!("{name}: {} element(s) coloured", coloured));
                    for rule in &set.rules {
                        ui.horizontal(|ui| {
                            let (rect, _) = ui
                                .allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                            ui.painter().rect_filled(
                                rect,
                                2.0,
                                egui::Color32::from_rgb(
                                    rule.colour[0],
                                    rule.colour[1],
                                    rule.colour[2],
                                ),
                            );
                            let mut text = crate::colour_sets::Set::describe(&rule.rule);
                            if rule.transparency > 0.0 {
                                text.push_str(&format!(
                                    ", {}% transparent",
                                    (rule.transparency * 100.0).round() as u32
                                ));
                            }
                            ui.label(text);
                        });
                    }
                }
                None => {
                    ui.label(format!(
                        "{name}: {coloured} element(s) coloured (the set is no longer in the \
                         colour-sets file)"
                    ));
                }
            }
        }
        if let Some(note) = &self.colour_note {
            match note {
                Ok(message) => {
                    ui.label(message);
                }
                Err(message) => {
                    ui.colored_label(palette.error, message);
                }
            }
        }
    }

    /// Evaluates the named set on a worker (one paged `query-elements` per rule) and applies
    /// the resulting overlay when it answers. The model is never written: the queries are the
    /// CLI's own read-only command.
    fn apply_colour(&mut self, name: &str) {
        if self.colour_job.is_some() || self.other_command_running_pre_colour() {
            self.colour_note = Some(Err("another command is running; wait for it.".into()));
            return;
        }
        let Some(set) = self.colour_sets.find(name).cloned() else {
            return;
        };
        let (cli, store) = (self.source.cli.clone(), self.source.store.clone());
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        let stats = Arc::clone(&self.stats);
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("beamcraft-colour".into())
            .spawn(move || {
                let _ = tx.send(crate::colour_sets::evaluate(
                    &cli, &store, &set, &flag, &stats,
                ));
            });
        match spawned {
            Ok(_) => {
                self.colour_note = None;
                self.colour_job = Some(ColourJob {
                    set: name.to_owned(),
                    rx,
                    cancel,
                });
            }
            Err(error) => {
                self.colour_note = Some(Err(format!(
                    "the colour worker could not be started: {error}"
                )));
            }
        }
    }

    /// The other commands worth waiting for, without the colour job itself (which is never
    /// running when an apply is requested from the panel).
    fn other_command_running_pre_colour(&self) -> bool {
        self.props_job.is_some()
            || self.publish_job.is_some()
            || self.edit_job.is_some()
            || self.job.is_some()
            || self.grid_job.is_some()
    }

    /// Clears the overlay and colours nothing again.
    fn clear_colour(&mut self) {
        if let Some(job) = &self.colour_job {
            job.cancel.store(true, Ordering::Relaxed);
        }
        self.colour_job = None;
        self.colour = None;
        self.colour_note = None;
        self.recolour();
    }

    /// Applies a finished evaluation. The controls are disabled while a job runs, so the answer
    /// is always the request the panel still shows.
    fn poll_colour(&mut self) {
        let Some(job) = &self.colour_job else { return };
        let received = match job.rx.try_recv() {
            Ok(received) => Some(received),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => None,
        };
        let set = job.set.clone();
        self.colour_job = None;
        let result = match received {
            Some(Ok(overlay)) if overlay.is_empty() => {
                Err(format!("no element in `{set}` matches: nothing to colour"))
            }
            Some(Ok(overlay)) => Ok(overlay),
            Some(Err(message)) => Err(message),
            None => Err("the colour worker ended without an answer".to_string()),
        };
        match result {
            Ok(overlay) => {
                self.colour_note =
                    Some(Ok(format!("`{set}` colours {} element(s).", overlay.len())));
                self.colour = Some((set, Arc::new(overlay)));
                self.recolour();
            }
            Err(message) => {
                self.colour = None;
                self.recolour();
                self.colour_note = Some(Err(message));
            }
        }
    }

    /// Rebuilds and re-uploads the loaded scene with the current overlay (V17). Display state
    /// only: the snapshot and its token are untouched, the meshes the device holds are reused,
    /// and only the instance buffers are sent again. A quiet no-op while nothing is loaded.
    fn recolour(&mut self) {
        let Some(mut loaded) = self.loaded.take() else {
            return;
        };
        let overlay = self.colour.as_ref().map(|(_, overlay)| Arc::clone(overlay));
        let scene = crate::scene::Scene::from_snapshot_reusing(
            &loaded.snapshot,
            beamcraft_engine::Representation::Full,
            &loaded.uploaded.resident(),
            overlay.as_deref(),
        );
        match scene.and_then(|scene| {
            let summary = scene.summary();
            Uploaded::upload(
                &self.state.device,
                &self.state.queue,
                &scene,
                Some(&loaded.uploaded),
            )
            .map(|uploaded| (summary, uploaded))
        }) {
            Ok((summary, uploaded)) => {
                loaded.summary = summary;
                loaded.uploaded = uploaded;
            }
            Err(error) => {
                // The device keeps the previous scene; the overlay is reported as refused.
                self.error = Some(format!("the coloured scene was not applied: {error}"));
            }
        }
        self.loaded = Some(loaded);
    }

    fn edit_panel(&mut self, ui: &mut egui::Ui) {
        ui.label("Click an element in the view to select it. Esc clears the selection.");
        match self.editor.selected() {
            Some(info) => {
                ui.label(format!("Selected: {} {}", info.kind, info.name));
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
                ui.checkbox(
                    &mut self.editor.copy_instance,
                    "Instance: share shape and material edits",
                );
                ui.label("Offset of the copy in mm:");
                point_fields(ui, &mut self.editor.copy_offset);
            }
            _ => {
                ui.label("Deletes the element (refused while another element links to it).");
            }
        });
        let busy = self.editor.busy();
        let busy = busy || self.other_command_running();
        let mut apply = false;
        let mut cancel = false;
        ui.horizontal(|ui| {
            apply = ui
                .add_enabled(
                    selected && !busy,
                    egui::Button::new(match action {
                        EditKind::Move => "Move",
                        EditKind::Copy => "Copy",
                        _ => "Delete",
                    }),
                )
                .clicked();
            cancel = ui.add_enabled(!busy, egui::Button::new("Cancel")).clicked();
        });
        if apply {
            let target = Target {
                store: &self.source.store,
                workspace: self.source.workspace.as_deref(),
            };
            match self.editor.preview(action, target) {
                Ok(_) => self.start_edit(None),
                Err(error) => {
                    self.editor.cancel();
                    self.edit_note = Some(Err(error.to_string()));
                }
            }
        }
        if cancel {
            self.editor.cancel();
            self.set_bench(workbench::Bench::Select);
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
                if created.kind == ToolKind::Plate {
                    self.plate_draft.clear();
                    self.plate_hover = None;
                    self.tools.plate.contour.clear();
                    self.tools.plate_plane = None;
                    self.distance_input.clear();
                    self.distance_aim = None;
                    self.plate_mode = self.bench == workbench::Bench::Plate;
                } else {
                    self.beam_mode = matches!(
                        self.bench,
                        workbench::Bench::Beam | workbench::Bench::Column
                    );
                }
                self.tool_note = Some(Ok(format!(
                    "Created {what}. Pick points for the next one, or press Esc to finish."
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
        form.contour.clear();
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
        // A mode phase measures its mode: the drawing style follows the driver, as a key press
        // would.
        if let Some(mode) = request.mode {
            self.mode = mode;
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

    fn install_plane(&mut self, plane: Option<beamcraft_engine::work_plane::WorkPlane>) {
        self.tools.cancel();
        self.tools.plane = plane;
        self.tools.plate_plane = None;
        self.distance_input.clear();
        self.distance_aim = None;
        self.plate_draft.clear();
        self.plate_hover = None;
        self.tools.plate.contour.clear();
        self.plane_pick = false;
        self.beam_mode = matches!(
            self.bench,
            workbench::Bench::Beam | workbench::Bench::Column
        );
        self.beam.reset();
        self.beam_hover = None;
        self.ghost_end = None;
        self.renderer
            .set_ghost(&self.state.device, &self.state.queue, None);
        self.tool_note = Some(Ok(
            "Work plane updated for this session; the model is unchanged.".into(),
        ));
    }

    /// One click of the placement: the engine's front-face point on the viewed snapshot, else
    /// the ground plane. The first click arms the start, the second fills the form's endpoints.
    /// The query is bounded by the snapshot caps and runs only while placing.
    fn place_beam(&mut self, ray: crate::edit::Ray) {
        if self.other_command_running() {
            return;
        }
        let point = match self.creation_point(ray, false) {
            Ok(point) => point,
            Err(error) => {
                self.clear_member_preview();
                self.tool_note = Some(Err(error.to_string()));
                return;
            }
        };
        self.accept_member_point(point);
    }

    /// Accept a resolved cursor or numeric endpoint through the same member command path.
    fn accept_member_point(&mut self, point: [f64; 3]) {
        let point = match self.beam.start() {
            Some(start) => match self.tools.plane.map_or_else(
                || self.tools.member.axis_lock.constrain(start, point),
                |plane| plane.constrain_world(start, point, self.tools.member.axis_lock),
            ) {
                Ok(point) => point,
                Err(e) => {
                    self.clear_member_preview();
                    self.tool_note = Some(Err(e.to_string()));
                    return;
                }
            },
            None => point,
        };
        self.distance_input.clear();
        self.distance_aim = None;
        self.beam_hover = None;
        match self.beam.click(point) {
            None => {
                self.clear_member_preview();
                self.tool_note = Some(Ok(format!(
                    "Start {}. Pick the end point.",
                    PointText(point)
                )));
            }
            Some((start, end)) => {
                let points = self.tools.plane.map(|plane| {
                    plane
                        .to_local(start)
                        .and_then(|start| plane.to_local(end).map(|end| (start, end)))
                });
                let (start, end) = match points {
                    Some(Ok(pair)) => pair,
                    Some(Err(e)) => {
                        self.clear_member_preview();
                        self.tool_note = Some(Err(e.to_string()));
                        return;
                    }
                    None => (start, end),
                };
                self.tools.member.set_endpoints(start, end);
                self.beam_mode = false;
                self.ghost_end = None;
                self.renderer
                    .set_ghost(&self.state.device, &self.state.queue, None);
                self.create_from_form();
            }
        }
    }

    /// Rebuilds the ghost preview for the pointer's hover point: the engine's own body mesh for
    /// start -> hover, where the pointer ray meets the plane through the start parallel to the
    /// view. Display only: an axis the engine refuses simply draws no ghost, and Commit still
    /// asks the engine.
    fn clear_member_preview(&mut self) {
        self.beam_hover = None;
        self.ghost_end = None;
        self.renderer
            .set_ghost(&self.state.device, &self.state.queue, None);
    }

    fn update_ghost(&mut self, view: &View, ndc: [f32; 2]) {
        let Some(start) = self.beam.start() else {
            return;
        };
        let origin_mm = self
            .loaded
            .as_ref()
            .map_or([0.0; 3], |l| l.summary.origin_mm);
        let Some(ray) = ray_through(view, ndc, origin_mm) else {
            self.clear_member_preview();
            return;
        };
        let Ok(end) = self.creation_point(ray, false) else {
            self.clear_member_preview();
            return;
        };
        let constrained = match self.tools.plane {
            Some(plane) => plane.constrain_world(start, end, self.tools.member.axis_lock),
            None => self.tools.member.axis_lock.constrain(start, end),
        };
        let Ok(end) = constrained else {
            self.clear_member_preview();
            return;
        };
        self.beam_hover = Some(end);
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
            curved: false,
            id: ElementId::parse("01990000-0000-7000-8000-00000000e347").ok()?,
            kind: match self.tools.member.kind {
                crate::tools::MemberKind::Beam => ModelMemberKind::Beam,
                crate::tools::MemberKind::Column => ModelMemberKind::Column,
            },
            name: "ghost".into(),
            class: 0,
            axis_points_mm: vec![start, end],
            profile: self.tools.member.profile.as_deref()?.parse().ok()?,
            roll_rad: self.tools.member.roll.trim().parse::<f64>().ok(),
            section_placement: None,
            placement: Placement {
                translation_mm: [0.0; 3],
                orientation_xyzw: orientation,
            },
            material: None,
            cuts: Vec::new(),
            hole_records: Vec::new(),
        };
        let body = steel::engine::member::evaluate_member(
            &authored,
            ProfileInfo::Rectangle {
                width_mm: rect[0],
                height_mm: rect[1],
                declared_area_mm2: None,
            },
        )
        .ok()?;
        // Ghost buffers are display-only world geometry; the committed body stays local.
        let solid = body.solid.rotated_by(authored.placement.orientation_xyzw)?;
        Some(GhostMesh::of(
            solid.vertices(),
            solid.triangles(),
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

    fn info(&mut self, ui: &mut egui::Ui) {
        ui.label(format!("{:.0} FPS", self.fps));
        ui.label(&self.adapter);
        ui.label(format!("{}x MSAA", self.renderer.samples));
        ui.label(format!("Render mode: {}", self.mode.as_token()));
        if let Some(note) = &self.shortcuts_note {
            ui.label(format!("Shortcuts: {note}"));
        }
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
        if let Some(appearance) = &mut self.appearance {
            appearance.poll(ui.ctx());
        }
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
                !wants_text
                    && (!self.plate_mode || self.plate_draft.points().is_empty())
                    && (!self.beam_mode || self.beam.start().is_none())
                    && i.key_pressed(egui::Key::Minus),
            )
        });
        self.poll_commit();
        self.poll_grid();
        self.poll_annotations();
        self.poll_edit();
        self.poll_props();
        self.poll_publish();
        self.poll_status();
        self.poll_select();
        self.poll_reload();
        self.poll_colour();
        self.selection_shortcuts(ui.ctx());
        self.render_mode_shortcuts(ui.ctx());
        self.perf_step(ui.ctx(), frame.info().cpu_usage);
        self.construction_keyboard(ui.ctx());
        if self.menu_open && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.menu_open = false;
        } else if !wants_text
            && !self.other_command_running()
            && ui.input(|i| i.key_pressed(egui::Key::Escape))
        {
            self.grid_line.click = None;
            self.plane_pick = false;
            // Esc closes the publish dialog first; it clears the selection only when none is open.
            if self.publisher.phase() == Phase::Confirming {
                self.publisher.cancel();
            } else {
                self.invalidate_select();
                self.editor.deselect();
                self.props.clear();
                self.set_bench(workbench::Bench::Select);
                self.grid_edit = None;
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
        if fit && let Some(l) = &self.loaded {
            let (target, extent) = self.grid_framing(&l.summary);
            if let Err(e) = self.camera.fit(target, extent, self.aspect()) {
                self.error = Some(e.to_string());
            }
        }
        if self.text {
            // The inspector wraps long values within the width chosen by the user.
            self.header(ui);
            let ctx = ui.ctx().clone();
            self.command_dialog(&ctx);
            if self.menu_open {
                self.menu_page(ui);
                return;
            }
            self.ribbon(ui);
            self.status_bar(ui);
            // Registered after the status bar, so the snap strip sits directly above it.
            self.inspector(ui);
            self.project_browser(ui);
            self.details_window(&ctx);
            self.publish_dialog(&ctx);
            self.views_window(&ctx);
            self.draw_view_windows(&ctx);
            self.sheet_editor.window(
                &ctx,
                &self.source.cli,
                self.source.actor.as_deref(),
                &self.stats,
            );
        }
        egui::CentralPanel::default().show(ui, |ui| {
            let size = ui.available_size();
            self.viewport_height = size.y.max(1.0);
            let scale = ui.ctx().pixels_per_point();
            let [width, height] = target_size([size.x, size.y], scale);
            if let Err(e) = self.resize(width, height) {
                self.error = Some(e.to_string());
            }
            let response = ui.add(egui::Image::new((self.texture, size)).sense(viewport_sense()));
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
            let mut picked_grid = None;
            match self
                .camera
                .navigate(orbit, pan, zoom)
                .and_then(|()| self.camera.view(aspect))
            {
                Ok(view) => {
                    let rgb = if ui.ctx().theme() == egui::Theme::Dark {
                        [0x17, 0x21, 0x2B]
                    } else {
                        [0xED, 0xF1, 0xF4]
                    };
                    let [r, g, b] = rgb.map(|c: u8| f64::from(c) / 255.0);
                    self.renderer.background = wgpu::Color { r, g, b, a: 1.0 };
                    if let Some(l) = &self.loaded {
                        // The display cut is uniform state: the scene and its snapshot never
                        // change. Planes are model millimetres; the uniform is scene-centred.
                        let clip = match &self.cut {
                            Some(cut) => crate::cut_plane::ClipGpu::of(cut, l.summary.origin_mm),
                            None => crate::cut_plane::ClipGpu::none(),
                        };
                        // One loaded scene, five drawing styles: the mode picks the pipeline and
                        // never rewrites the snapshot or its token.
                        if self.mode == RenderMode::SelectedOnly {
                            // The one engine query runs on a selection or reload change, not
                            // per frame; the fields are disjoint, so the cache updates while
                            // the scene is borrowed.
                            let key = (
                                self.editor
                                    .selected()
                                    .map(|s| s.id.clone())
                                    .unwrap_or_default(),
                                l.summary.origin_mm,
                            );
                            if self.selected_only.0 != key {
                                let draw = self
                                    .editor
                                    .selected()
                                    .and_then(|s| beamcraft_engine::ElementId::parse(&s.id).ok())
                                    .and_then(|id| {
                                        crate::scene::SelectedDraw::of(
                                            &l.snapshot,
                                            &id,
                                            l.summary.origin_mm,
                                        )
                                    });
                                self.selected_only = (key, draw);
                            }
                        }
                        match self.mode {
                            RenderMode::SelectedOnly => match &self.selected_only.1 {
                                Some(selected) => {
                                    self.renderer.draw_one(
                                        &self.state.device,
                                        &self.state.queue,
                                        &l.uploaded,
                                        view,
                                        &self.stats,
                                        &selected.digest,
                                        &selected.instance,
                                        &clip,
                                    );
                                }
                                // Nothing selected: an empty view, not the whole model.
                                None => {
                                    self.renderer.draw(
                                        &self.state.device,
                                        &self.state.queue,
                                        &l.uploaded,
                                        view,
                                        &self.stats,
                                        RenderMode::SelectedOnly,
                                        &clip,
                                    );
                                }
                            },
                            mode => {
                                self.renderer.draw(
                                    &self.state.device,
                                    &self.state.queue,
                                    &l.uploaded,
                                    view,
                                    &self.stats,
                                    mode,
                                    &clip,
                                );
                            }
                        }
                    }
                    if let Some(l) = &self.loaded {
                        if self.grids_visible {
                            self.grid_scene.paint(
                                ui,
                                response.rect,
                                &view,
                                l.summary.origin_mm,
                                self.grid_edit.as_ref().map(|edit| edit.id.as_str()),
                            );
                        }
                        if let Ok(rows) = self.annotation.rows.as_deref()
                            && !rows.is_empty()
                        {
                            annotation_panel::paint(
                                ui,
                                response.rect,
                                &view,
                                l.summary.origin_mm,
                                rows,
                            );
                        }
                        if let Some(selected) = self.editor.selected() {
                            let key = (selected.id.clone(), l.snapshot.token().as_str().to_owned());
                            if self.selection_edges.0 != key {
                                let edges = beamcraft_engine::ElementId::parse(&selected.id)
                                    .ok()
                                    .and_then(|id| {
                                        beamcraft_engine::query::feature::element_edges(
                                            &l.snapshot,
                                            &id,
                                        )
                                        .ok()
                                    })
                                    .unwrap_or_default();
                                self.selection_edges = (key, edges);
                            }
                            let painter = ui.painter_at(response.rect);
                            draw_selection(
                                &painter,
                                response.rect,
                                &view,
                                l.summary.origin_mm,
                                &self.selection_edges.1,
                            );
                        } else {
                            self.selection_edges = Default::default();
                        }
                    }
                    let finished_creation = self.finish_creation_gesture(&response, &view);
                    // A closure gesture consumes the click before it can append another point.
                    if !finished_creation
                        && response.clicked_by(egui::PointerButton::Primary)
                        && let Some(pos) = response.interact_pointer_pos()
                        && let Some(ndc) = ndc_of(
                            [pos.x, pos.y],
                            [response.rect.min.x, response.rect.min.y],
                            [response.rect.width(), response.rect.height()],
                        )
                        && let Some(ray) = ray_through(
                            &view,
                            ndc,
                            self.loaded
                                .as_ref()
                                .map_or([0.0; 3], |l| l.summary.origin_mm),
                        )
                    {
                        // The grids switch (V11) takes grids out of picking; the display
                        // switch stays `grids_visible`.
                        if !self.beam_mode
                            && !self.plate_mode
                            && self.grid_line.click.is_none()
                            && self.grids_visible
                            && self
                                .selection
                                .is_on(beamcraft_engine::query::pick::SelectionClass::Grids)
                        {
                            picked_grid = self
                                .grid_scene
                                .hit_test(
                                    response.rect,
                                    &view,
                                    self.loaded
                                        .as_ref()
                                        .map_or([0.0; 3], |l| l.summary.origin_mm),
                                    pos,
                                )
                                .map(|grid| grid.id.clone());
                        }
                        if picked_grid.is_none() {
                            picked = Some(ray);
                        }
                    }
                    if self.plate_mode {
                        let resolved = response
                            .hover_pos()
                            .and_then(|pos| {
                                ndc_of(
                                    [pos.x, pos.y],
                                    [response.rect.min.x, response.rect.min.y],
                                    [response.rect.width(), response.rect.height()],
                                )
                            })
                            .and_then(|ndc| {
                                ray_through(
                                    &view,
                                    ndc,
                                    self.loaded
                                        .as_ref()
                                        .map_or([0.0; 3], |l| l.summary.origin_mm),
                                )
                            })
                            .and_then(|ray| self.creation_hit(ray, true).ok());
                        self.plate_hover = resolved.map(|(point, _)| point);
                        self.snap_hover = resolved.and_then(|(_, kind)| kind);
                        self.paint_plate_draft(ui, response.rect, &view);
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
                        self.paint_member_draft(ui, response.rect, &view);
                    }
                }
                Err(e) => self.error = Some(e.to_string()),
            }
            if let Some(id) = picked_grid {
                if !self.other_command_running() {
                    self.select_grid(&id);
                }
            } else if let Some(ray) = picked {
                if self.plane_pick {
                    if let Some(loaded) = &self.loaded {
                        match beamcraft_engine::work_plane::pick_work_plane(
                            &loaded.snapshot,
                            ray.origin,
                            ray.direction,
                        ) {
                            Ok(Some(plane)) => {
                                self.plane_form.set(plane);
                                self.install_plane(Some(plane));
                            }
                            Ok(None) => {
                                self.tool_note =
                                    Some(Err("No face was hit; pick a model face.".into()))
                            }
                            Err(e) => self.tool_note = Some(Err(e.to_string())),
                        }
                    }
                } else if self.grid_line.click.is_some() {
                    self.place_grid_line(ray);
                } else if self.plate_mode {
                    self.place_plate(ray);
                } else if self.beam_mode {
                    self.place_beam(ray);
                } else if self.bench == workbench::Bench::Annotations {
                    self.annotation_click(ray);
                } else if !self.other_command_running() {
                    self.start_select(ray);
                }
            }
        });
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        App, Source, WATCH_INTERVAL, load_icon, needs_refit, revisions_token, spawn_watch,
        target_size,
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
    use beamcraft_engine::{Limits, RenderMode, Representation, ViewPin, ViewSnapshot};
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
            config_dir: None,
        };
        let stats = Arc::new(crate::stats::Stats::default());
        Some(App::new(&cc, source, stats).unwrap())
    }

    #[test]
    fn an_applied_overlay_rebuilds_the_scene_and_leaves_the_snapshot_token() {
        let Some(mut app) = headless() else { return };
        let pin = ViewPin::new(Sha256::digest(b"revision").unwrap(), 1);
        let snapshot = Arc::new(
            ViewSnapshot::build(pin, plate_scene(3, false).unwrap(), &Limits::default()).unwrap(),
        );
        let scene = Scene::from_snapshot(&snapshot, Representation::Full).unwrap();
        let uploaded =
            crate::render::Uploaded::new(&app.state.device, &app.state.queue, &scene, &app.stats)
                .unwrap();
        app.loaded = Some(super::Loaded {
            snapshot: Arc::clone(&snapshot),
            summary: scene.summary(),
            uploaded,
        });
        let ids: std::collections::BTreeSet<beamcraft_model::id::ElementId> =
            snapshot.elements().iter().map(|e| e.id).collect();
        app.colour = Some((
            "Test".into(),
            Arc::new(crate::colour_sets::Overlay::from_rules(vec![(
                ids,
                [0.0, 1.0, 0.0, 1.0],
            )])),
        ));
        let token = snapshot.token().clone();
        app.recolour();
        // The loaded snapshot is exactly the one shown before: the overlay is display state
        // applied to the device, never a new reading of the model.
        assert_eq!(app.loaded.as_ref().unwrap().snapshot.token(), &token);
        assert!(app.colour.is_some());
    }

    #[test]
    fn applying_a_set_without_a_cli_reports_and_colours_nothing() {
        let Some(mut app) = headless() else { return };
        app.apply_colour(crate::colour_sets::CLASS_SET_NAME);
        assert!(app.colour_job.is_some());
        for _ in 0..250 {
            app.poll_colour();
            if app.colour_job.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(app.colour_job.is_none(), "the missing CLI answers quickly");
        assert!(app.colour.is_none());
        assert!(matches!(app.colour_note, Some(Err(_))));
    }

    #[test]
    fn manage_rectangular_grid_opens_form_and_cancel_does_not_change_snapshot() {
        let Some(mut app) = headless() else { return };
        deliver(&mut app, refreshed(2));
        let token = app.loaded.as_ref().unwrap().snapshot.token().clone();
        app.ribbon_tab = super::workbench::RibbonTab::Manage;
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        assert!(super::inter(&ctx));
        let mut frame = |events| {
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1400.0, 900.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    app.header(ui);
                    app.ribbon(ui);
                    app.status_bar(ui);
                    app.inspector(ui);
                    app.project_browser(ui);
                    egui::CentralPanel::default().show(ui, |_| {});
                },
            );
            let labels: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape {
                        Some((
                            text.galley.text().to_owned(),
                            text.pos + text.galley.size() / 2.0,
                        ))
                    } else {
                        None
                    }
                })
                .collect();
            output.drop_without_applying_deltas();
            labels
        };
        let click = |pos| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        let labels = frame(vec![]);
        let grid = labels
            .iter()
            .find(|(text, _)| text == "Rectangular grid")
            .unwrap()
            .1;
        frame(click(grid));
        let labels = frame(vec![]);
        for expected in [
            "U family · X positions",
            "V family · Y positions",
            "Create grid",
            "Cancel",
        ] {
            assert!(
                labels.iter().any(|(text, _)| text == expected),
                "missing {expected}"
            );
        }
        let cancel = labels.iter().find(|(text, _)| text == "Cancel").unwrap().1;
        frame(click(cancel));
        frame(vec![]);
        assert_eq!(app.bench, super::workbench::Bench::Select);
        assert!(app.grid_job.is_none());
        assert_eq!(app.loaded.as_ref().unwrap().snapshot.token(), &token);
    }

    #[test]
    fn workspace_has_dense_entity_and_snap_strips_without_theme_or_shortcut_text() {
        let Some(mut app) = headless() else { return };
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        assert!(crate::app::inter(&ctx));
        for preference in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
            ctx.set_theme(preference);
            for width in [1100.0, 1400.0] {
                let mut viewport_bottom = 0.0;
                let output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 800.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        app.header(ui);
                        app.ribbon(ui);
                        app.status_bar(ui);
                        app.inspector(ui);
                        app.project_browser(ui);
                        egui::CentralPanel::default().show(ui, |ui| {
                            viewport_bottom = ui.max_rect().bottom();
                        });
                    },
                );
                let labels: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| {
                        if let egui::Shape::Text(text) = &shape.shape {
                            Some(text.galley.text().to_owned())
                        } else {
                            None
                        }
                    })
                    .collect();
                let bounds: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|clipped| {
                        if let egui::Shape::Rect(shape) = &clipped.shape {
                            (shape.rect.size() == egui::vec2(24.0, 24.0)
                                && shape.rect.top() >= 748.0)
                                .then_some((shape.rect, clipped.clip_rect))
                        } else {
                            None
                        }
                    })
                    .collect();
                output.drop_without_applying_deltas();
                assert!(labels.iter().any(|label| label == "Find a command"));
                assert!(!labels.iter().any(|label| label.contains("Ctrl K")
                    || matches!(label.as_str(), "Theme" | "System" | "Light" | "Dark")));
                assert_eq!(bounds.len(), 34, "20 entity + 14 snap switches: {bounds:?}");
                assert!(
                    bounds.iter().all(|(rect, clip)| rect.left() >= 0.0
                        && rect.right() <= width
                        && clip.contains_rect(*rect)
                        && rect.top() >= viewport_bottom),
                    "toolbar switches clipped: {bounds:?}"
                );
                for (index, (a, _)) in bounds.iter().enumerate() {
                    assert!(
                        bounds
                            .iter()
                            .skip(index + 1)
                            .all(|(b, _)| !a.intersects(*b)),
                        "toolbar switches overlap"
                    );
                }
            }
        }
    }

    #[test]
    fn menu_uses_reference_categories_flat_commands_and_real_new_navigation() {
        let Some(mut app) = headless() else { return };
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        assert!(crate::app::inter(&ctx));
        app.menu_open = true;
        app.from_library = true;
        let mut frame = |events| {
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    app.header(ui);
                    app.menu_page(ui);
                },
            );
            let labels: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape {
                        Some((
                            text.galley.text().to_owned(),
                            text.pos + text.galley.size() / 2.0,
                        ))
                    } else {
                        None
                    }
                })
                .collect();
            output.drop_without_applying_deltas();
            labels
        };
        let labels = frame(vec![]);
        for expected in [
            "Menu",
            "New",
            "Open",
            "Catalogs",
            "Profile catalog",
            "Material catalog",
            "Editors",
            "Diagnose & repair",
            "Exit BeamCraft",
        ] {
            assert!(
                labels.iter().any(|(label, _)| label == expected),
                "Missing menu item {expected}"
            );
        }
        assert!(!labels.iter().any(|(label, _)| label == "Fit model"));
        let pos = labels.iter().find(|(label, _)| label == "New").unwrap().1;
        frame(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        frame(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert!(app.return_to_library && app.return_to_create);
    }

    #[test]
    fn command_search_takes_keyboard_focus_and_escape_closes_it() {
        let Some(mut app) = headless() else { return };
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        assert!(crate::app::inter(&ctx));
        app.command_search = true;
        app.command_focus = true;
        let mut frame = |events| {
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    events,
                    ..Default::default()
                },
                |_| app.command_dialog(&ctx),
            );
            output.drop_without_applying_deltas();
        };
        frame(vec![]);
        frame(vec![]);
        frame(vec![egui::Event::Text("fit".into())]);
        frame(vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert_eq!(app.command_query, "fit");
        assert!(!app.command_search);
    }

    /// A refresh that read `scene`.
    fn reloaded(scene: Result<Refreshed, Error>) -> Reloaded {
        Reloaded {
            scene,
            definitions: Err(ToolError::local("NOT_FOUND", "no store in this test")),
            grids: Ok(crate::grid::GridScene::default()),
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
    fn interactive_plate_and_member_creation_use_real_commands_without_preview_steps() {
        let mut app =
            headless().expect("a software/native adapter is required for modelling acceptance");
        let executable = std::env::current_exe().unwrap();
        let cli = executable
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(format!("beamcraft{}", std::env::consts::EXE_SUFFIX));
        assert!(
            cli.is_file(),
            "build beamcraft-cli before native modelling tests"
        );
        let root = std::env::temp_dir().join(format!(
            "bc-interactive-{}-{}",
            std::process::id(),
            beamcraft_model::digest::Sha256::digest(
                format!("{:?}", std::time::SystemTime::now()).as_bytes()
            )
            .unwrap()
        ));
        std::fs::create_dir_all(&root).unwrap();
        app.source.store = root.join("model");
        std::fs::create_dir_all(&app.source.store).unwrap();
        app.source.cli = cli.clone();
        app.loaded = None; // The first plate must work in an entirely empty model.
        let ray = |x, y| crate::edit::Ray {
            origin: [x, y, 5000.0],
            direction: [0.0, 0.0, -1.0],
        };
        let run = |store: &std::path::Path, args: &[&str]| {
            let output = std::process::Command::new(&cli)
                .args(args)
                .args(["--store", store.to_str().unwrap()])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            beamcraft_json::decode(&output.stdout, &beamcraft_json::Limits::DEFAULT).unwrap()
        };
        let wait = |app: &mut App| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while app.job.is_some() && std::time::Instant::now() < deadline {
                app.poll_commit();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(app.job.is_none(), "creation timed out");
            assert!(app.last_commit_ok, "{:?}", app.tool_note);
        };
        app.set_bench(super::workbench::Bench::Plate);
        app.place_plate(ray(0.0, 0.0));
        app.place_plate(ray(4000.0, 0.0));
        assert!(app.job.is_none());
        app.set_bench(super::workbench::Bench::Select); // Cancelling publishes nothing.
        assert!(!app.source.store.join("revisions").exists());
        assert!(app.plate_draft.points().is_empty());
        app.set_bench(super::workbench::Bench::Plate);
        for [x, y] in [[0.0, 0.0], [4000.0, 0.0], [4000.0, 3000.0], [0.0, 3000.0]] {
            app.place_plate(ray(x, y));
        }
        assert!(app.job.is_none());
        app.finish_plate();
        assert!(app.job.is_some());
        wait(&mut app);
        let inspected = run(&app.source.store, &["inspect-elements"]);
        assert_eq!(inspected.get("plates").unwrap().array().unwrap().len(), 1);
        assert!(app.plate_draft.points().is_empty());
        let profile = run(
            &app.source.store,
            &["define-profile", "--width", "100", "--height", "200"],
        );
        app.tools.member.profile = Some(
            profile
                .get("definition")
                .unwrap()
                .get("digest")
                .unwrap()
                .string()
                .unwrap()
                .into(),
        );
        app.set_bench(super::workbench::Bench::Beam);
        app.place_beam(ray(10000.0, 10000.0));
        assert!(app.job.is_none());
        app.place_beam(ray(15000.0, 10000.0));
        assert!(
            app.job.is_some(),
            "second point must create without Preview/Commit"
        );
        wait(&mut app);
        let inspected = run(&app.source.store, &["inspect-elements"]);
        assert_eq!(inspected.get("members").unwrap().array().unwrap().len(), 1);
        let ctx = egui::Context::default();
        super::inter(&ctx);
        crate::theme::install(&ctx);
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 900.0),
                )),
                ..Default::default()
            },
            |ui| {
                app.tools_panel(ui);
            },
        );
        let labels = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Text(t) = &shape.shape {
                    Some(t.galley.text().to_owned())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert!(!labels.iter().any(|text| text.contains("create-member")
            || text.contains("--store")
            || text == "Commit"
            || text == "Preview"));
        output.drop_without_applying_deltas();
        let member_id = inspected.get("members").unwrap().array().unwrap()[0]
            .get("element_id")
            .unwrap()
            .string()
            .unwrap();
        let mut details = crate::command::element_details(
            &cli,
            &app.source.store,
            member_id,
            &crate::stats::Stats::default(),
        )
        .unwrap();
        details.quantities.stored_volume_freshness = Some("stale".into());
        let stamp = details.revision.clone();
        app.props.load(member_id, Ok(details));
        app.props.observe_head(Some(stamp));
        app.set_bench(super::workbench::Bench::Select);
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 1100.0),
                )),
                ..Default::default()
            },
            |ui| {
                app.props_panel(ui);
                app.property_actions(ui);
            },
        );
        let labels = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Text(t) = &shape.shape {
                    Some(t.galley.text().to_owned())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for expected in ["Apply", "Reset", "Level", "Not available"] {
            assert!(
                labels.iter().any(|label| label == expected),
                "missing {expected}: {labels:?}"
            );
        }
        assert!(
            labels
                .iter()
                .any(|text| text.contains("stored value out of date"))
        );
        assert!(!labels.iter().any(|text| text.contains("Command preview")
            || text.contains("Preview command")
            || text.contains("Revision and quantities")
            || text.contains(member_id)
            || text.contains("--")));
        output.drop_without_applying_deltas();
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn typed_length_and_o_create_a_spatial_plate_through_real_keyboard_events() {
        let mut app =
            headless().expect("a native/software adapter is required for construction input");
        let executable = std::env::current_exe().unwrap();
        let cli = executable
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(format!("beamcraft{}", std::env::consts::EXE_SUFFIX));
        assert!(cli.is_file(), "build beamcraft-cli before input acceptance");
        let root = std::env::temp_dir().join(format!(
            "bc-length-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        app.source.cli = cli;
        app.source.store = root.clone();
        app.loaded = None;
        app.set_bench(super::workbench::Bench::Plate);
        let ctx = egui::Context::default();
        let frame = |app: &mut App, mut events: Vec<egui::Event>| {
            let released = events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::Key {
                        key,
                        physical_key,
                        pressed: true,
                        modifiers,
                        ..
                    } => Some(egui::Event::Key {
                        key: *key,
                        physical_key: *physical_key,
                        pressed: false,
                        repeat: false,
                        modifiers: *modifiers,
                    }),
                    _ => None,
                })
                .collect::<Vec<_>>();
            events.extend(released);
            ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| app.construction_keyboard(ui.ctx()),
            )
            .drop_without_applying_deltas()
        };
        let key = |key, repeat| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat,
            modifiers: egui::Modifiers::NONE,
        };
        app.plate_draft.push([0.; 3]).unwrap();
        app.plate_hover = Some([3., 4., 12.]);
        frame(
            &mut app,
            vec![
                egui::Event::Text("500".into()),
                key(egui::Key::Enter, false),
            ],
        );
        let p = *app.plate_draft.points().last().unwrap();
        assert!((p.iter().map(|v| v * v).sum::<f64>().sqrt() - 500.).abs() < 1e-9);
        assert!(
            app.job.is_none(),
            "numeric Enter adds a vertex, never finishes the plate"
        );
        frame(&mut app, vec![key(egui::Key::Enter, true)]);
        assert_eq!(app.plate_draft.points().len(), 2);
        assert!(app.job.is_none());
        app.plate_draft.clear();
        app.plate_draft.push([0.; 3]).unwrap();
        frame(&mut app, vec![key(egui::Key::O, false)]);
        assert!(app.selection.orthogonal);
        // The same orthogonal engine resolver used by cursor preview selects a genuine Z ray.
        let z = beamcraft_engine::coordinate::orthogonal_cursor(
            [0.; 3],
            [1000., -1000., 1000.],
            [-1000., 1000., -400.],
            None,
        )
        .unwrap();
        app.plate_hover = Some(z);
        frame(
            &mut app,
            vec![
                egui::Event::Text("500".into()),
                key(egui::Key::Enter, false),
            ],
        );
        assert_eq!(app.plate_draft.points().last(), Some(&[0., 0., 500.]));
        app.plate_hover = Some([200., 2., 500.]);
        frame(
            &mut app,
            vec![
                egui::Event::Text("300".into()),
                key(egui::Key::Enter, false),
            ],
        );
        assert_eq!(app.plate_draft.points().last(), Some(&[300., 0., 500.]));
        app.plate_hover = Some([300., 0., -10.]);
        frame(
            &mut app,
            vec![
                egui::Event::Text("500".into()),
                key(egui::Key::Enter, false),
            ],
        );
        assert_eq!(app.plate_draft.points().last(), Some(&[300., 0., 0.]));
        assert!(
            app.tools.plane.is_none(),
            "contour frame must not replace the session frame"
        );
        frame(&mut app, vec![key(egui::Key::Enter, false)]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while app.job.is_some() && std::time::Instant::now() < deadline {
            app.poll_commit();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            app.job.is_none() && app.last_commit_ok,
            "{:?}",
            app.tool_note
        );
        assert!(app.tools.plane.is_none());
        let output = std::process::Command::new(&app.source.cli)
            .args(["inspect-elements", "--store", root.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(output.status.success());
        let json =
            beamcraft_json::decode(&output.stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
        let plate = &json.get("plates").unwrap().array().unwrap()[0];
        assert_eq!(
            plate
                .get("quantities")
                .unwrap()
                .get("volume")
                .unwrap()
                .get("mm3")
                .unwrap()
                .float()
                .unwrap(),
            1_500_000.
        );
        let profile = std::process::Command::new(&app.source.cli)
            .args([
                "define-profile",
                "--store",
                root.to_str().unwrap(),
                "--width",
                "100",
                "--height",
                "200",
            ])
            .output()
            .unwrap();
        assert!(profile.status.success());
        let profile =
            beamcraft_json::decode(&profile.stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
        app.tools.member.profile = Some(
            profile
                .get("definition")
                .unwrap()
                .get("digest")
                .unwrap()
                .string()
                .unwrap()
                .into(),
        );
        app.set_bench(super::workbench::Bench::Beam);
        app.selection.orthogonal = false;
        app.loaded = None;
        app.place_beam(crate::edit::Ray {
            origin: [0., 0., 5000.],
            direction: [0., 0., -1.],
        });
        app.beam_hover = Some([3., 4., 12.]);
        // A viewport/button focus is not a text field and must not swallow length input.
        ctx.memory_mut(|m| m.request_focus(egui::Id::unique("viewport-input-regression")));
        frame(
            &mut app,
            vec![
                egui::Event::Text("10000".into()),
                key(egui::Key::Backspace, false),
            ],
        );
        assert_eq!(app.distance_input, "1000");
        assert_eq!(app.distance_aim, Some([3., 4., 12.]));
        // Moving the cursor while typing keeps the direction captured by the first digit.
        app.beam_hover = Some([100., 0., 0.]);
        frame(&mut app, vec![key(egui::Key::Enter, false)]);
        assert!(app.job.is_some(), "{:?}", app.tool_note);
        frame(&mut app, vec![key(egui::Key::Enter, true)]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while app.job.is_some() && std::time::Instant::now() < deadline {
            app.poll_commit();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            app.last_commit_ok && app.job.is_none(),
            "{:?}",
            app.tool_note
        );
        let output = std::process::Command::new(&app.source.cli)
            .args(["inspect-elements", "--store", root.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(output.status.success());
        let json =
            beamcraft_json::decode(&output.stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
        let members = json.get("members").unwrap().array().unwrap();
        assert_eq!(members.len(), 1);
        let axis = members[0].get("axis_points_mm").unwrap().array().unwrap();
        let start = axis[0].array().unwrap();
        let end = axis[1].array().unwrap();
        let delta = start
            .iter()
            .zip(end)
            .map(|(a, b)| b.float().unwrap() - a.float().unwrap())
            .collect::<Vec<_>>();
        assert!((delta.iter().map(|v| v * v).sum::<f64>().sqrt() - 1000.).abs() < 1e-8);
        assert!(
            delta
                .iter()
                .zip([3000. / 13., 4000. / 13., 12000. / 13.])
                .all(|(a, b)| (a - b).abs() < 1e-8)
        );
        drop(app);
        assert!(root.is_absolute() && root.file_name().is_some());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bay_midpoint_preview_and_triangle_selection_use_real_exported_geometry() {
        use beamcraft_engine::query::{feature::element_edges, pick::SnapKind};
        let mut app =
            headless().expect("a native/software adapter is required for construction acceptance");
        let executable = std::env::current_exe().unwrap();
        let cli = executable
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(format!("beamcraft{}", std::env::consts::EXE_SUFFIX));
        let root = std::env::temp_dir().join(format!(
            "bc-bay-outline-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let run = |command: &str, args: &[&str]| {
            let out = std::process::Command::new(&cli)
                .arg(command)
                .args(["--store", root.to_str().unwrap()])
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stdout)
            );
            beamcraft_json::decode(&out.stdout, &beamcraft_json::Limits::DEFAULT).unwrap()
        };
        run(
            "create-grid",
            &[
                "--name",
                "Bays",
                "--u-expression",
                "0 1000 2000",
                "--v-expression",
                "0 800 1200",
            ],
        );
        let plate = run(
            "create-plate",
            &[
                "--contour",
                "0,0;0,800;1000,0",
                "--normalize-winding",
                "true",
                "--thickness",
                "10",
                "--translation",
                "5000,5000,0",
            ],
        );
        let id = beamcraft_engine::ElementId::parse(
            plate
                .get("plate")
                .unwrap()
                .get("element_id")
                .unwrap()
                .string()
                .unwrap(),
        )
        .unwrap();
        app.source.cli = cli.clone();
        app.source.store = root.clone();
        let reload = crate::source::reload(
            &cli,
            &root,
            &crate::source::Known::default(),
            None,
            &app.stats,
        );
        deliver(&mut app, reload);
        app.set_bench(super::workbench::Bench::Plate);
        for kind in SnapKind::ALL {
            app.selection.set_snap(kind, kind == SnapKind::Midpoint);
        }
        app.selection.snap_nearest = false;
        let origin = app.loaded.as_ref().unwrap().summary.origin_mm;
        let local = std::array::from_fn(|i| ([500., 0., 0.][i] - origin[i]) as f32);
        app.camera.fit(local, 2000., app.aspect()).unwrap();
        for orthographic in [false, true] {
            app.camera.set_projection(orthographic);
            app.plate_draft.clear();
            let view = app.camera.view(app.aspect()).unwrap();
            let [x, y, _] = view.project(local);
            let ray = crate::edit::ray_through(
                &view,
                [x + 4. / (app.viewport_height * app.aspect()), y],
                origin,
            )
            .unwrap();
            let (point, kind) = app.creation_hit(ray, true).unwrap();
            assert_eq!(kind, Some(SnapKind::Midpoint));
            assert!(
                point
                    .iter()
                    .zip([500., 0., 0.])
                    .all(|(a, b)| (a - b).abs() < 1e-7)
            );
            app.place_plate(ray);
            assert_eq!(app.plate_draft.points(), &[point]);
        }
        let edges = element_edges(&app.loaded.as_ref().unwrap().snapshot, &id).unwrap();
        assert_eq!(edges.len(), 9);
        let view = crate::camera::Camera::new([5500., 5400., 0.], 2000., 1.)
            .unwrap()
            .view(1.)
            .unwrap();
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 800.));
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(rect),
                ..Default::default()
            },
            |ui| {
                super::draw_selection(ui.painter(), rect, &view, [0.; 3], &edges);
            },
        );
        let lines = output
            .shapes
            .iter()
            .filter_map(|s| match s.shape {
                egui::Shape::LineSegment { points, .. } => Some(points),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(lines.len(), 9);
        // The absent fourth corner of the triangle must never become a highlight endpoint.
        let [x, y, _] = view.project([6000., 5800., 0.]);
        let phantom = egui::pos2((x + 1.) * 400., (1. - y) * 400.);
        assert!(lines.iter().flatten().all(|p| p.distance(phantom) > 1.));
        output.drop_without_applying_deltas();
        drop(app);
        assert!(root.is_absolute() && root.file_name().is_some());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn plate_cursor_snaps_follow_pixels_switches_and_the_real_exported_geometry() {
        use beamcraft_engine::query::pick::SnapKind;
        let mut app =
            headless().expect("a native/software adapter is required for snap acceptance");
        let executable = std::env::current_exe().unwrap();
        let cli = executable
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(format!("beamcraft{}", std::env::consts::EXE_SUFFIX));
        assert!(cli.is_file(), "build beamcraft-cli before snap acceptance");
        let root = std::env::temp_dir().join(format!(
            "bc-snaps-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let run = |command: &str, args: &[&str]| {
            let out = std::process::Command::new(&cli)
                .args([command, "--store", root.to_str().unwrap()])
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stdout)
            );
            beamcraft_json::decode(&out.stdout, &beamcraft_json::Limits::DEFAULT).unwrap()
        };
        run(
            "create-grid",
            &[
                "--name",
                "G",
                "--u-expression",
                "0 1000",
                "--v-expression",
                "0 2000",
            ],
        );
        run(
            "create-plate",
            &[
                "--width",
                "1000",
                "--length",
                "800",
                "--thickness",
                "10",
                "--translation",
                "3000,0,0",
            ],
        );
        let profile = run(
            "define-profile",
            &[
                "--family", "i_shape", "--h", "200", "--b", "100", "--tw", "6", "--tf", "10",
                "--r", "0",
            ],
        );
        let profile = profile
            .get("definition")
            .unwrap()
            .get("digest")
            .unwrap()
            .string()
            .unwrap();
        run(
            "create-member",
            &[
                "--start",
                "0,4000,0",
                "--end",
                "3000,4000,0",
                "--profile-digest",
                profile,
                "--section-plane-offset",
                "350",
            ],
        );
        app.source.cli = cli.clone();
        app.source.store = root.clone();
        let reload = crate::source::reload(
            &cli,
            &root,
            &crate::source::Known::default(),
            None,
            &app.stats,
        );
        assert!(
            reload
                .grids
                .as_ref()
                .is_ok_and(|scene| scene.member_axes.len() == 1)
        );
        deliver(&mut app, reload);
        let token = app.loaded.as_ref().unwrap().snapshot.token().clone();
        app.set_bench(super::workbench::Bench::Plate);
        let only = |app: &mut App, kind| {
            for k in SnapKind::ALL {
                app.selection.set_snap(k, k == kind);
            }
        };
        let ray_at = |app: &App, point: [f64; 3], pixels: f32| {
            let origin = app.loaded.as_ref().unwrap().summary.origin_mm;
            let local = std::array::from_fn(|i| (point[i] - origin[i]) as f32);
            let view = app.camera.view(app.aspect()).unwrap();
            let [x, y, _] = view.project(local);
            let ndc = [x + pixels * 2.0 / (app.viewport_height * app.aspect()), y];
            crate::edit::ray_through(&view, ndc, origin).unwrap()
        };
        // All defaults enabled: a closer grid line must not steal the crossing.
        for zoom in [0.0, 0.7, -0.7] {
            app.camera.navigate([0.0; 2], [0.0; 2], zoom).unwrap();
            let ray = ray_at(&app, [1000.0, 2000.0, 0.0], 3.0);
            assert_eq!(
                app.creation_hit(ray, true).unwrap(),
                ([1000., 2000., 0.], Some(SnapKind::GridIntersection))
            );
            app.plate_draft.clear();
            app.place_plate(ray);
            assert_eq!(app.plate_draft.points(), &[[1000., 2000., 0.]]);
        }
        app.plate_draft.clear();
        only(&mut app, SnapKind::GridIntersection);
        let ray = ray_at(&app, [1000., 2000., 0.], 3.0);
        app.grids_visible = false;
        assert!(app.creation_hit(ray, true).unwrap().1.is_none());
        app.grids_visible = true;
        app.selection.snapping_enabled = false;
        assert!(app.creation_hit(ray, true).unwrap().1.is_none());
        app.selection.snapping_enabled = true;
        // Source switches do not enable a point kind. In particular, grid ON cannot
        // resurrect nearest points when the hourglass switch is OFF.
        let previous_camera = app.camera;
        let origin = app.loaded.as_ref().unwrap().summary.origin_mm;
        let target = std::array::from_fn(|i| ([0., 611., 0.][i] - origin[i]) as f32);
        app.camera.fit(target, 1500., app.aspect()).unwrap();
        for k in SnapKind::ALL {
            app.selection.set_snap(k, true);
        }
        for reference in [false, true] {
            for geometry in [false, true] {
                for grids in [false, true] {
                    for nearest in [false, true] {
                        app.selection.snap_reference = reference;
                        app.selection.snap_geometry = geometry;
                        app.selection.snap_grids = grids;
                        app.selection.snap_nearest = nearest;
                        let ray = ray_at(&app, [0., 611., 0.], 0.0);
                        let (_, kind) = app.creation_hit(ray, true).unwrap();
                        assert_eq!(
                            kind,
                            if reference && grids && nearest {
                                Some(SnapKind::GridLine)
                            } else {
                                None
                            },
                            "reference={reference} geometry={geometry} grids={grids} nearest={nearest}"
                        );
                        app.grid_line.click = Some(false);
                        app.place_grid_line(crate::edit::Ray {
                            origin: [1., 611., 1000.],
                            direction: [0., 0., -1.],
                        });
                        assert_eq!(
                            app.grid_line.points[0],
                            if reference && grids && nearest {
                                ["0".to_owned(), "611".to_owned()]
                            } else {
                                ["1".to_owned(), "611".to_owned()]
                            }
                        );
                    }
                }
            }
        }
        app.camera = previous_camera;
        app.grid_line.click = None;
        app.selection.snap_reference = true;
        app.selection.snap_geometry = true;
        app.selection.snap_grids = true;
        app.selection.snap_nearest = true;
        only(&mut app, SnapKind::GridIntersection);
        let far = ray_at(&app, [1000., 2000., 0.], 18.0);
        assert!(app.creation_hit(far, true).unwrap().1.is_none());
        for (kind, p) in [
            (SnapKind::Midpoint, [3500., 0., 0.]),
            (SnapKind::Vertex, [3000., 0., 0.]),
            (SnapKind::Edge, [3600., 0., 0.]),
            (SnapKind::Center, [1500., 4000., 0.]),
            (SnapKind::AxisEnd, [0., 4000., 0.]),
        ] {
            only(&mut app, kind);
            let ray = ray_at(&app, p, 0.0);
            let (point, snap) = app.creation_hit(ray, true).unwrap();
            assert_eq!(snap, Some(kind));
            assert!(
                point.iter().zip(p).all(|(a, b)| (a - b).abs() < 0.01),
                "{kind:?}: {point:?}"
            );
        }
        assert_eq!(app.loaded.as_ref().unwrap().snapshot.token(), &token);
        assert!(app.job.is_none());
        // Every clicked vertex is exactly a grid intersection and Finish publishes those points.
        only(&mut app, SnapKind::GridIntersection);
        for p in [
            [0., 0., 0.],
            [1000., 0., 0.],
            [1000., 2000., 0.],
            [0., 2000., 0.],
        ] {
            let ray = ray_at(&app, p, 2.0);
            app.place_plate(ray);
        }
        assert_eq!(app.plate_draft.points().len(), 4);
        app.finish_plate();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while app.job.is_some() && std::time::Instant::now() < deadline {
            app.poll_commit();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            app.job.is_none() && app.last_commit_ok,
            "{:?}",
            app.tool_note
        );
        let inspected = run("inspect-elements", &[]);
        assert!(
            inspected
                .get("plates")
                .unwrap()
                .array()
                .unwrap()
                .iter()
                .any(|plate| {
                    plate
                        .get("quantities")
                        .unwrap()
                        .get("volume")
                        .unwrap()
                        .get("mm3")
                        .unwrap()
                        .float()
                        .unwrap()
                        == 20_000_000.
                })
        );
        drop(app);
        assert!(root.is_absolute() && root.file_name().is_some());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn review_triangle_and_rectangle_finish_with_right_or_wheel_click_and_drags_do_not_finish() {
        use crate::tools::{Created, ToolError};
        let mut app = headless().expect("software/native graphics required for input acceptance");
        let exe = std::env::current_exe().unwrap();
        let cli = exe
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(format!("beamcraft{}", std::env::consts::EXE_SUFFIX));
        assert!(cli.is_file());
        let root = std::env::temp_dir().join(format!(
            "bc-finish-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        app.source.cli = cli.clone();
        app.source.store = root.clone();
        app.loaded = None;
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(600.0, 600.0));
        let view = crate::camera::Camera::new([0.0; 3], 10_000.0, 1.0)
            .unwrap()
            .view(1.0)
            .unwrap();
        for kind in beamcraft_engine::query::pick::SnapKind::ALL {
            app.selection.set_snap(kind, false);
        }
        let frame = |app: &mut App, events| {
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, 900.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
                    app.finish_creation_gesture(&response, &view);
                },
            );
            output.drop_without_applying_deltas();
        };
        let event = |button, pos, pressed| egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut count = 0;
        for (vertices, button, near_first) in [
            (3, egui::PointerButton::Secondary, false),
            (3, egui::PointerButton::Middle, false),
            (4, egui::PointerButton::Secondary, false),
            (4, egui::PointerButton::Middle, false),
            (3, egui::PointerButton::Primary, true),
        ] {
            app.set_bench(super::workbench::Bench::Plate);
            let mut points = if vertices == 3 {
                vec![[0.0, 0.0, 0.0], [4000.0, 0.0, 0.0], [0.0, 3000.0, 0.0]]
            } else {
                vec![
                    [0.0, 0.0, 0.0],
                    [4000.0, 0.0, 0.0],
                    [4000.0, 3000.0, 0.0],
                    [0.0, 3000.0, 0.0],
                ]
            };
            if button == egui::PointerButton::Secondary {
                points.reverse();
            }
            for p in points {
                app.plate_draft.push(p).unwrap();
            }
            let pos = if near_first {
                rect.center() + egui::vec2(8.0, 0.0)
            } else {
                rect.center() + egui::vec2(100.0, 100.0)
            };
            frame(&mut app, vec![egui::Event::PointerMoved(pos)]);
            frame(&mut app, vec![event(button, pos, true)]);
            frame(&mut app, vec![event(button, pos, false)]);
            assert!(
                app.job.is_some(),
                "{vertices} vertices, {button:?} must save: {:?}",
                app.tool_note
            );
            assert_eq!(app.plate_draft.points().len(), vertices);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while app.job.is_some() && std::time::Instant::now() < deadline {
                app.poll_commit();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(
                app.last_commit_ok && app.job.is_none(),
                "{:?}",
                app.tool_note
            );
            count += 1;
        }
        let output = std::process::Command::new(&cli)
            .args(["inspect-elements", "--store", root.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(output.status.success());
        let inspected =
            beamcraft_json::decode(&output.stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
        assert_eq!(
            inspected.get("plates").unwrap().array().unwrap().len(),
            count
        );
        app.set_bench(super::workbench::Bench::Plate);
        for p in [[0.0, 0.0, 0.0], [4000.0, 0.0, 0.0], [0.0, 3000.0, 0.0]] {
            app.plate_draft.push(p).unwrap();
        }
        let a = rect.center();
        let b = a + egui::vec2(80.0, 20.0);
        frame(&mut app, vec![egui::Event::PointerMoved(a)]);
        frame(&mut app, vec![event(egui::PointerButton::Middle, a, true)]);
        frame(&mut app, vec![egui::Event::PointerMoved(b)]);
        frame(&mut app, vec![event(egui::PointerButton::Middle, b, false)]);
        assert!(app.job.is_none(), "panning must not finish the contour");
        assert_eq!(app.plate_draft.points().len(), 3);
        // Tool changes may not discard the draft/evidence while saving.
        app.tools.plate.contour = app.plate_draft.contour(None).unwrap();
        app.tools.preview(&app.source.store).unwrap();
        let (_tx, rx) = std::sync::mpsc::channel::<Result<Created, ToolError>>();
        app.job = Some(rx);
        app.set_bench(super::workbench::Bench::Select);
        assert_eq!(app.bench, super::workbench::Bench::Plate);
        assert!(app.tools.staged().is_some());
        assert_eq!(app.plate_draft.points().len(), 3);
        app.job = None;
        // A failed point resolution must discard both visible preview and its cache.
        app.tools.plane =
            Some(beamcraft_engine::work_plane::WorkPlane::parallel("xy", 1e9).unwrap());
        app.beam.click([0.0; 3]);
        app.beam_hover = Some([1.0; 3]);
        app.ghost_end = Some([1; 3]);
        app.update_ghost(&view, [0.0; 2]);
        assert!(app.beam_hover.is_none() && app.ghost_end.is_none());
        for button in [egui::PointerButton::Secondary, egui::PointerButton::Middle] {
            app.set_bench(super::workbench::Bench::Beam);
            app.beam.click([0.0; 3]);
            frame(&mut app, vec![egui::Event::PointerMoved(a)]);
            frame(&mut app, vec![event(button, a, true)]);
            frame(&mut app, vec![event(button, a, false)]);
            assert_eq!(app.bench, super::workbench::Bench::Select);
            assert!(app.beam.start().is_none() && app.job.is_none());
        }
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn review_duplicate_choices_have_distinct_labels_without_changing_unique_labels() {
        let levels: Vec<(String, String, f64)> = vec![
            (
                "01990000-0000-7000-8000-000000000001".into(),
                "Floor".into(),
                3000.0,
            ),
            (
                "01990000-0000-7000-8000-000000000002".into(),
                "Floor".into(),
                3000.0,
            ),
        ];
        assert_ne!(
            super::level_choice_label(&levels[0].0, &levels),
            super::level_choice_label(&levels[1].0, &levels)
        );
        let choices = vec![
            crate::tools::Choice {
                digest: "a000000000000001".into(),
                label: "Steel (0001)".into(),
                rect_mm: None,
            },
            crate::tools::Choice {
                digest: "b000000000000001".into(),
                label: "Steel (0002)".into(),
                rect_mm: None,
            },
        ];
        assert_ne!(
            super::definition_choice_label(&choices[0], &choices),
            super::definition_choice_label(&choices[1], &choices)
        );
        assert_eq!(
            super::definition_choice_label(&choices[0], &choices[..1]),
            "Steel"
        );
    }

    #[test]
    fn drawn_grid_clicks_use_engine_snaps_and_cancel_without_a_command() {
        use beamcraft_engine::query::pick::SnapKind;
        use beamcraft_model::grid::{DrawnGridLine, Grid, GridFamily};
        use beamcraft_model::id::ElementId;
        let mut app = headless().expect("a native or software wgpu adapter is required");
        deliver(&mut app, refreshed(1));
        let token = app.loaded.as_ref().unwrap().snapshot.token().clone();
        let id = ElementId::parse("01990000-0000-7000-8000-000000001001").unwrap();
        app.grid_scene.query_grids = vec![Grid {
            id,
            name: "Drawn".into(),
            translation_mm: [0.0, 0.0],
            orientation_zw: [0.0, 1.0],
            u: GridFamily::Drawn(vec![DrawnGridLine {
                axis_id: ElementId::parse("01990000-0000-7000-8000-000000001002").unwrap(),
                label: "D1".into(),
                start_mm: [0.0, 0.0],
                end_mm: [3000.0, 4000.0],
            }]),
            v: GridFamily::Drawn(vec![DrawnGridLine {
                axis_id: ElementId::parse("01990000-0000-7000-8000-000000001003").unwrap(),
                label: "D2".into(),
                start_mm: [3000.0, 0.0],
                end_mm: [0.0, 4000.0],
            }]),
        }];
        for kind in SnapKind::ALL {
            app.selection
                .set_snap(kind, kind == SnapKind::GridIntersection);
        }
        let ray = crate::edit::Ray {
            origin: [1503.0, 2002.0, 1000.0],
            direction: [0.0, 0.0, -1.0],
        };
        app.grid_line.click = Some(false);
        app.place_grid_line(ray);
        assert_eq!(app.grid_line.points[0], ["1500", "2000"]);
        assert_eq!(app.grid_line.click, Some(true));
        app.grids_visible = false;
        app.place_grid_line(ray);
        assert_eq!(app.grid_line.points[1], ["1503", "2002"]);
        assert_eq!(app.grid_line.click, None);
        assert!(app.grid_job.is_none());
        assert_eq!(app.loaded.as_ref().unwrap().snapshot.token(), &token);
        app.grid_line.click = Some(false);
        app.set_bench(super::workbench::Bench::Select);
        assert!(app.grid_line.click.is_none());
        assert!(app.grid_job.is_none());
    }

    #[test]
    fn grid_tool_opens_its_form_and_cancel_keeps_the_snapshot() {
        let Some(mut app) = headless() else { return };
        app.ribbon_tab = super::workbench::RibbonTab::Manage;
        deliver(&mut app, refreshed(2));
        let token = app.loaded.as_ref().unwrap().snapshot.token().clone();
        app.beam_mode = true;
        app.beam.click([100.0, 200.0, 0.0]);
        app.ghost_end = Some([100, 200, 0]);
        let ctx = egui::Context::default();
        super::inter(&ctx);
        crate::theme::install(&ctx);
        let mut draw = |events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1400.0, 900.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    app.selection_shortcuts(ui.ctx());
                    app.header(ui);
                    app.ribbon(ui);
                    app.inspector(ui);
                    egui::CentralPanel::default().show(ui, |_| {});
                },
            );
            // Only layout and input are inspected; this test does not upload textures.
            output.textures_delta.clear();
            output
        };
        let position = |output: &egui::FullOutput, label: &str| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text == label => {
                        Some(text.pos + text.galley.size() / 2.0)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{label} was not drawn"))
        };
        let click = |pos| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        let first = draw(Vec::new());
        let form = draw(click(position(&first, "Rectangular grid")));
        position(&form, "Create grid");
        assert!(!form.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "Show grids")));
        draw(click(position(&form, "Cancel")));
        let cancelled = draw(Vec::new());
        position(&cancelled, "Properties");
        assert_eq!(app.bench, super::workbench::Bench::Select);
        assert!(app.grid_job.is_none());
        assert!(!app.beam_mode && app.beam.start().is_none() && app.ghost_end.is_none());
        assert!(app.grids_visible);
        assert_eq!(app.loaded.as_ref().unwrap().snapshot.token(), &token);
    }

    #[test]
    fn the_dense_switches_keep_upstream_selection_and_snap_behavior() {
        let Some(mut app) = headless() else { return };
        deliver(&mut app, refreshed(2));
        let token = app.loaded.as_ref().unwrap().snapshot.token().clone();
        let ctx = egui::Context::default();
        super::inter(&ctx);
        crate::theme::install(&ctx);
        let draw = |app: &mut App, events| {
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1400.0, 900.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| app.status_bar(ui),
            );
            let rects: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) if rect.rect.size() == egui::vec2(24.0, 24.0) => {
                        Some(rect.rect)
                    }
                    _ => None,
                })
                .collect();
            let snap = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Snap" => {
                    Some(text.pos + text.galley.size() / 2.0)
                }
                _ => None,
            });
            output.drop_without_applying_deltas();
            (rects, snap)
        };
        let click = |pos| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        let (rects, snap) = draw(&mut app, vec![]);
        assert_eq!(rects.len(), 34);
        draw(&mut app, click(rects[2].center()));
        draw(&mut app, click(rects[7].center()));
        use beamcraft_engine::query::pick::{SelectionClass, SnapKind};
        assert_eq!(
            app.selection.filter().off_classes(),
            vec![SelectionClass::Parts, SelectionClass::Grids]
        );
        assert!(app.selection.snapping_enabled);
        draw(&mut app, click(snap.unwrap()));
        assert!(!app.selection.snapping_enabled);
        draw(&mut app, click(rects[20].center()));
        assert!(!app.selection.snap_on(SnapKind::GridIntersection));
        draw(&mut app, click(rects[21].center())); // one endpoint type, three source families
        for kind in [SnapKind::Vertex, SnapKind::AxisEnd, SnapKind::GridEnd] {
            assert!(!app.selection.snap_on(kind));
        }
        draw(&mut app, click(rects[21].center()));
        // Toolbar source gates and point kinds retain independent state.
        draw(&mut app, click(rects[24].center())); // nearest/hourglass
        assert!(!app.selection.snap_nearest);
        assert!(app.selection.snap_grids);
        assert!(!app.selection.snap_kinds().contains(&SnapKind::GridLine));
        draw(&mut app, click(rects[29].center())); // grid source
        assert!(!app.selection.snap_grids);
        assert!(!app.selection.snap_nearest);
        draw(&mut app, click(rects[30].center())); // construction source (T02)
        assert!(!app.selection.snap_construction);
        draw(&mut app, click(rects[32].center())); // reference master
        draw(&mut app, click(rects[33].center())); // physical master
        assert!(
            !app.selection.snap_construction
                && !app.selection.snap_reference
                && !app.selection.snap_geometry
        );
        draw(&mut app, click(snap.unwrap())); // global snap back on
        assert!(app.selection.snap_kinds().is_empty());
        assert_eq!(app.loaded.as_ref().unwrap().snapshot.token(), &token);
    }

    #[test]
    fn ctrl_1_to_5_and_the_ribbon_switch_render_modes_without_touching_the_scene() {
        let Some(mut app) = headless() else { return };
        deliver(&mut app, refreshed(2));
        let token = app.loaded.as_ref().unwrap().snapshot.token().clone();
        let ctx = egui::Context::default();
        super::inter(&ctx);
        crate::theme::install(&ctx);
        let draw = |app: &mut App, events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1400.0, 900.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    app.render_mode_shortcuts(ui.ctx());
                    app.header(ui);
                    app.ribbon(ui);
                },
            );
            output.textures_delta.clear();
            output
        };
        // egui tracks modifiers only through ModifiersChanged, as winit delivers them.
        let press = |key, ctrl| {
            let modifiers = if ctrl {
                egui::Modifiers::CTRL
            } else {
                egui::Modifiers::NONE
            };
            vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                },
            ]
        };
        let position = |output: &egui::FullOutput, label: &str| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text == label => {
                        Some(text.pos + text.galley.size() / 2.0)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{label} was not drawn"))
        };
        let click = |pos| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        // A bare digit never switches a mode; Ctrl+1 to Ctrl+5 do, in order.
        draw(&mut app, press(egui::Key::Num1, false));
        assert_eq!(app.mode, RenderMode::Shaded);
        for (key, expected) in [
            (egui::Key::Num1, RenderMode::Wireframe),
            (egui::Key::Num2, RenderMode::Transparent),
            (egui::Key::Num3, RenderMode::Grey),
            (egui::Key::Num4, RenderMode::Shaded),
            (egui::Key::Num5, RenderMode::SelectedOnly),
        ] {
            draw(&mut app, press(key, true));
            assert_eq!(app.mode, expected, "{key:?}");
        }
        assert_eq!(
            app.loaded.as_ref().unwrap().snapshot.token(),
            &token,
            "a mode switch never reloads the scene"
        );
        // A remapped binding replaces the default one; the other modes keep theirs.
        app.shortcuts
            .set(
                RenderMode::Wireframe,
                crate::shortcuts::Binding::parse("Ctrl+7").unwrap(),
            )
            .unwrap();
        draw(&mut app, press(egui::Key::Num1, true));
        assert_eq!(app.mode, RenderMode::SelectedOnly);
        draw(&mut app, press(egui::Key::Num7, true));
        assert_eq!(app.mode, RenderMode::Wireframe);
        // The View tab's DISPLAY buttons set the same state a key press does.
        let first = draw(&mut app, Vec::new());
        draw(&mut app, click(position(&first, "View")));
        let view_tab = draw(&mut app, Vec::new());
        draw(&mut app, click(position(&view_tab, "Transparent")));
        assert_eq!(app.mode, RenderMode::Transparent);
    }

    #[test]
    fn a_rejected_grid_framing_keeps_the_old_scene_and_its_guides_together() {
        let Some(mut app) = headless() else { return };
        deliver(&mut app, refreshed(2));
        let token = app.loaded.as_ref().unwrap().snapshot.token().clone();
        let old_guides = app.grid_scene.clone();
        let mut next = refreshed(3);
        next.grids = Ok(crate::grid::GridScene {
            lines: vec![crate::grid::GridLine {
                label: "A".into(),
                points: [[-1e7, 0.0, 0.0], [1e7, 0.0, 0.0]],
            }],
            ..Default::default()
        });
        deliver(&mut app, next);
        assert!(app.error.is_some());
        assert_eq!(app.loaded.as_ref().unwrap().snapshot.token(), &token);
        assert_eq!(app.grid_scene, old_guides);
    }

    #[test]
    fn selected_grid_properties_reset_and_reject_stale_edits_without_writing() {
        let Some(mut app) = headless() else { return };
        deliver(&mut app, refreshed(1));
        let token = app.loaded.as_ref().unwrap().snapshot.token().clone();
        let id = "01990000-0000-7000-8000-000000000001";
        let form = crate::grid::GridForm::default();
        app.grid_scene = crate::grid::GridScene {
            lines: vec![crate::grid::GridLine {
                label: "1".into(),
                points: [[0.0, -2000.0, 0.0], [0.0, 12000.0, 0.0]],
            }],
            grids: vec![crate::grid::GridProperties {
                id: id.into(),
                form: form.clone(),
                editable: true,
                lines: 0..1,
            }],
            ..Default::default()
        };
        app.select_grid(id);
        assert_eq!(app.bench, super::workbench::Bench::Select);
        assert!(app.editor.selected().is_none());
        let ctx = egui::Context::default();
        super::inter(&ctx);
        crate::theme::install(&ctx);
        let draw = |app: &mut App, events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1400.0, 900.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    app.selection_shortcuts(ui.ctx());
                    app.header(ui);
                    app.ribbon(ui);
                    app.inspector(ui);
                    egui::CentralPanel::default().show(ui, |_| {});
                },
            );
            output.textures_delta.clear();
            output
        };
        let position = |output: &egui::FullOutput, label: &str| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text == label => {
                        Some(text.pos + text.galley.size() / 2.0)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{label} was not drawn"))
        };
        let click = |pos| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        draw(&mut app, Vec::new());
        app.grid_form.name = "Unapplied name".into();
        let edited = draw(&mut app, Vec::new());
        position(&edited, "Grid properties");
        for removed in ["Fit grid extents", "Reload properties", "Show grids"] {
            assert!(!edited.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == removed)));
        }
        draw(&mut app, click(position(&edited, "Cancel")));
        assert!(app.grid_edit.is_none());
        app.select_grid(id);
        assert_eq!(app.grid_form, form);
        app.grid_form.name = "My draft".into();
        app.grid_scene.grids[0].form.name = "External change".into();
        app.sync_grid_selection();
        let stale = draw(&mut app, Vec::new());
        assert!(stale.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.contains("changed while you were editing"))));
        draw(&mut app, click(position(&stale, "Apply changes")));
        assert!(app.grid_job.is_none());
        assert_eq!(app.grid_form.name, "My draft");
        app.select_grid(id);
        assert_eq!(app.grid_form.name, "External change");
        // Delete edits text while a field has focus, and deletes the grid otherwise.
        let current = draw(&mut app, Vec::new());
        let name_input = position(&current, "Name") + egui::vec2(0.0, 25.0);
        draw(&mut app, click(name_input));
        let key = |pressed| egui::Event::Key {
            key: egui::Key::Delete,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        draw(&mut app, vec![key(true)]);
        assert!(app.edit_job.is_none());
        ctx.memory_mut(|memory| {
            if let Some(focused) = memory.focused() {
                memory.surrender_focus(focused);
            }
        });
        draw(&mut app, vec![key(false)]);
        draw(&mut app, vec![key(true)]);
        assert!(app.edit_job.is_some());
        assert_eq!(app.editor.selected().unwrap().id, id);
        app.grid_scene.grids.clear();
        app.sync_grid_selection();
        assert!(app.grid_edit.is_none());
        assert!(app.grid_job.is_none());
        assert_eq!(app.loaded.as_ref().unwrap().snapshot.token(), &token);
    }

    #[test]
    fn inspector_can_be_widened_and_narrowed_without_editing_the_model() {
        let Some(mut app) = headless() else { return };
        deliver(&mut app, refreshed(1));
        let token = app.loaded.as_ref().unwrap().snapshot.token().clone();
        let ctx = egui::Context::default();
        super::inter(&ctx);
        crate::theme::install(&ctx);
        let draw = |app: &mut App, events| {
            let mut viewport = egui::Rect::NOTHING;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1400.0, 900.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    app.inspector(ui);
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ui, |ui| viewport = ui.max_rect());
                },
            );
            output.textures_delta.clear();
            viewport
        };
        let pointer = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        draw(&mut app, Vec::new());
        let initial = draw(&mut app, Vec::new());
        let mut current = initial;
        for delta in [-180.0, 120.0] {
            let start = egui::pos2(current.right(), 400.0);
            let end = start + egui::vec2(delta, 0.0);
            draw(
                &mut app,
                vec![egui::Event::PointerMoved(start), pointer(start, true)],
            );
            draw(&mut app, vec![egui::Event::PointerMoved(end)]);
            draw(&mut app, vec![pointer(end, false)]);
            let next = draw(&mut app, Vec::new());
            assert!(
                (next.right() - current.right() - delta).abs() < 3.0,
                "{current:?} -> {next:?}"
            );
            current = next;
        }
        assert_eq!(app.loaded.as_ref().unwrap().snapshot.token(), &token);
        assert!(!app.other_command_running());
    }

    #[test]
    fn ribbon_tabs_and_menu_remain_reachable_without_changing_the_model() {
        let Some(mut app) = headless() else { return };
        deliver(&mut app, refreshed(2));
        let token = app.loaded.as_ref().unwrap().snapshot.token().clone();
        let ctx = egui::Context::default();
        super::inter(&ctx);
        crate::theme::install(&ctx);
        for width in [760.0, 1100.0, 1600.0] {
            for tab in [
                super::workbench::RibbonTab::Steel,
                super::workbench::RibbonTab::Edit,
                super::workbench::RibbonTab::View,
                super::workbench::RibbonTab::Manage,
            ] {
                app.ribbon_tab = tab;
                let output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 900.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        app.header(ui);
                        app.ribbon(ui);
                    },
                );
                let menu = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "Menu" => {
                        Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                    }
                    _ => None,
                });
                output.drop_without_applying_deltas();
                let menu = menu.unwrap();
                assert!(menu.left() >= 0.0 && menu.right() <= width && menu.bottom() < 90.0);
            }
        }
        assert_eq!(app.loaded.as_ref().unwrap().snapshot.token(), &token);
        assert!(!app.other_command_running());
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

    /// A frame that draws the workspace panels and the saved-views window, returning the
    /// centre of every drawn text label and every filled rectangle (for the view-window
    /// frame, whose close button sits at its right edge).
    fn views_frame(
        app: &mut App,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        labels: &mut Vec<(String, egui::Pos2)>,
        rects: &mut Vec<egui::Rect>,
    ) {
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 900.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                app.ribbon(ui);
                app.project_browser(ui);
                app.views_window(ui.ctx());
                app.draw_view_windows(ui.ctx());
                egui::CentralPanel::default().show(ui, |_| {});
            },
        );
        for shape in &output.shapes {
            match &shape.shape {
                egui::Shape::Text(text) => labels.push((
                    text.galley.text().to_owned(),
                    text.pos + text.galley.size() / 2.0,
                )),
                egui::Shape::Rect(rect) if rect.fill != egui::Color32::TRANSPARENT => {
                    rects.push(rect.rect)
                }
                _ => {}
            }
        }
        output.drop_without_applying_deltas();
    }

    fn click(pos: egui::Pos2) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    #[test]
    fn a_saved_view_restores_the_camera_and_never_changes_the_model() {
        let Some(mut app) = headless() else { return };
        let dir = std::env::temp_dir().join(format!("beamcraft-v12-ui-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        app.source.store = dir.clone();
        deliver(&mut app, refreshed(2));
        let token = app.loaded.as_ref().unwrap().snapshot.token().clone();
        let ctx = egui::Context::default();
        super::inter(&ctx);
        crate::theme::install(&ctx);
        // The ribbon's View tab opens the saved-views window: a real click path.
        let mut labels = Vec::new();
        let mut rects = Vec::new();
        views_frame(&mut app, &ctx, vec![], &mut labels, &mut rects);
        let tab = labels
            .iter()
            .find(|(text, _)| text == "View")
            .map(|(_, pos)| *pos)
            .unwrap_or_else(|| panic!("the View tab was not drawn"));
        views_frame(&mut app, &ctx, click(tab), &mut labels, &mut rects);
        let open = labels
            .iter()
            .find(|(text, _)| text == "Saved views")
            .map(|(_, pos)| *pos)
            .unwrap_or_else(|| panic!("the Saved views button was not drawn"));
        views_frame(&mut app, &ctx, click(open), &mut labels, &mut rects);
        assert!(app.views_panel.window_open, "the window did not open");

        // The window's own buttons dispatch: saving the current view is one click there.
        app.views_panel.name = "Kept".into();
        let mut save = None;
        for _ in 0..3 {
            save = labels
                .iter()
                .find(|(text, _)| text == "Save current view")
                .map(|(_, pos)| *pos);
            if save.is_some() {
                break;
            }
            views_frame(&mut app, &ctx, vec![], &mut labels, &mut rects);
        }
        let save = save.unwrap_or_else(|| panic!("the save button was not drawn: {labels:?}"));
        views_frame(&mut app, &ctx, click(save), &mut labels, &mut rects);
        assert_eq!(app.views.len(), 1, "the window button did not dispatch");
        assert!(crate::views::session_path(&dir).is_file());
        let saved = app.views.iter().next().unwrap().clone();
        app.camera.navigate([1.0, -0.5], [80.0, 0.0], 0.7).unwrap();
        assert_ne!(app.camera.pose(), saved.pose);
        app.view_action(crate::views::Action::Apply("Kept".into()));
        assert_eq!(app.camera.pose(), saved.pose);
        assert_eq!(app.loaded.as_ref().unwrap().snapshot.token(), &token);

        // A work area is a view property: set, saved, dropped, restored.
        app.view_action(crate::views::Action::SetWorkArea(
            beamcraft_engine::views::WorkArea::new([0.0; 3], [4000.0, 3000.0, 2000.0]).unwrap(),
        ));
        app.views_panel.name = "Boxed".into();
        app.view_action(crate::views::Action::SaveCurrent);
        app.view_action(crate::views::Action::ClearWorkArea);
        assert!(app.work_area.is_none());
        app.view_action(crate::views::Action::Apply("Boxed".into()));
        assert_eq!(
            app.work_area,
            Some(
                beamcraft_engine::views::WorkArea::new([0.0; 3], [4000.0, 3000.0, 2000.0]).unwrap()
            )
        );
        assert_eq!(app.loaded.as_ref().unwrap().snapshot.token(), &token);

        // The sidecar round-trips into a fresh book, and delete persists.
        let book = crate::views::load(&dir).unwrap();
        assert_eq!(book.len(), 2);
        app.view_action(crate::views::Action::Delete("Kept".into()));
        assert_eq!(crate::views::load(&dir).unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn plane_views_resolve_from_displayed_grids_and_windows_cap_at_three() {
        let Some(mut app) = headless() else { return };
        let dir = std::env::temp_dir().join(format!("beamcraft-v12-planes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        app.source.store = dir.clone();
        deliver(&mut app, refreshed(2));
        // The displayed grids come from the same pinned snapshot; a drawn line "M" along +y.
        let grid = beamcraft_model::grid::Grid {
            id: beamcraft_engine::element::ElementId::parse("019a7bf0-0000-7000-8000-000000000123")
                .unwrap(),
            name: "Main".into(),
            translation_mm: [0.0, 0.0],
            orientation_zw: [0.0, 1.0],
            u: beamcraft_model::grid::GridFamily::Drawn(vec![
                beamcraft_model::grid::DrawnGridLine {
                    axis_id: beamcraft_engine::element::ElementId::parse(
                        "019a7bf0-0000-7000-8000-000000000124",
                    )
                    .unwrap(),
                    label: "M".into(),
                    start_mm: [1000.0, 0.0],
                    end_mm: [1000.0, 9000.0],
                },
            ]),
            v: beamcraft_model::grid::GridFamily::Drawn(vec![]),
        };
        app.grid_scene.query_grids = vec![grid];
        let centre = app.frame_centre();
        app.views_panel.name = "Along M".into();
        app.view_action(crate::views::Action::SavePlane(
            beamcraft_engine::views::PlaneSpec::GridLine {
                grid: "Main".into(),
                line: "M".into(),
                reverse: false,
            },
        ));
        let view = app.views.iter().next().unwrap();
        let pose = view.pose;
        // Level view at the line's midpoint, facing the plane normal (-x).
        assert_eq!(pose.target_mm, [1000.0, 4500.0, centre[2]]);
        assert_eq!(pose.pitch, 0.0);
        assert!((pose.yaw - std::f64::consts::PI).abs() < 1e-9);
        assert!(app.views_note.is_none(), "{:?}", app.views_note);
        // Extra windows are display-only and bounded; the real close button releases the slot.
        for _ in 0..5 {
            app.open_view_window();
        }
        assert_eq!(app.view_windows.len(), crate::views::MAX_WINDOWS);
        // Closing a window releases its slot on the next drawn frame.
        let ctx = egui::Context::default();
        super::inter(&ctx);
        crate::theme::install(&ctx);
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |_| {});
            },
        )
        .drop_without_applying_deltas();
        let mut labels = Vec::new();
        let mut rects = Vec::new();
        let mut title = None;
        for _ in 0..3 {
            labels.clear();
            rects.clear();
            views_frame(&mut app, &ctx, vec![], &mut labels, &mut rects);
            title = labels
                .iter()
                .find(|(text, _)| text == "View 2")
                .map(|(_, pos)| *pos);
            if title.is_some() {
                break;
            }
        }
        let title = title.unwrap_or_else(|| panic!("the view window did not draw: {labels:?}"));
        // The close button sits at the right edge of the window's own frame, the smallest
        // filled rectangle around the title that is wide enough to be a window.
        let frame = rects
            .iter()
            .filter(|r| r.contains(title) && r.width() > 300.0)
            .min_by(|a, b| a.area().total_cmp(&b.area()))
            .copied()
            .unwrap_or_else(|| panic!("no window frame around {title:?} in {rects:?}"));
        let close = egui::pos2(frame.right() - 11.0, title.y);
        views_frame(&mut app, &ctx, click(close), &mut labels, &mut rects);
        views_frame(&mut app, &ctx, vec![], &mut labels, &mut rects);
        assert_eq!(
            app.view_windows.len(),
            crate::views::MAX_WINDOWS - 1,
            "the close button did not release the window"
        );
        std::fs::remove_dir_all(&dir).unwrap();
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
    fn orbit_drag_moves_the_near_surface_with_the_pointer() {
        for preset in [Preset::Mouse, Preset::Trackpad] {
            for delta in [
                egui::vec2(-12.0, 0.0),
                egui::vec2(12.0, 0.0),
                egui::vec2(0.0, -12.0),
                egui::vec2(0.0, 12.0),
            ] {
                let mut camera = Camera::new([0.0; 3], 1000.0, 1.5).unwrap();
                let view = camera.view(1.5).unwrap();
                // A point on the near face of a model centred on the orbit target.
                let point = [
                    view.back[0] * 250.0,
                    view.back[1] * 250.0,
                    view.back[2] * 250.0,
                ];
                let before = view.project(point);
                let (orbit, pan, zoom) = navigate(
                    preset,
                    delta,
                    (preset == Preset::Trackpad, false, preset == Preset::Mouse),
                    preset == Preset::Mouse,
                    false,
                    camera.distance(),
                    true,
                    0.0,
                    1.0,
                    KEYS_NONE,
                    false,
                );
                camera.navigate(orbit, pan, zoom).unwrap();
                let after = camera.view(1.5).unwrap().project(point);
                if delta.x != 0.0 {
                    assert!((after[0] - before[0]) * delta.x > 0.0);
                } else {
                    // Projection uses Y up; pointer events use Y down.
                    assert!((after[1] - before[1]) * delta.y < 0.0);
                }
            }
        }
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
    fn orbit_drags_the_front_of_the_model_with_the_pointer_in_both_presets() {
        for preset in [Preset::Mouse, Preset::Trackpad] {
            for delta in [
                egui::vec2(10.0, 0.0),
                egui::vec2(-10.0, 0.0),
                egui::vec2(0.0, 10.0),
                egui::vec2(0.0, -10.0),
            ] {
                let mut camera = Camera::new([0.0; 3], 1000.0, 1.0).unwrap();
                let view = camera.view(1.0).unwrap();
                let [x, y, z, _] = view.back;
                let point = [x * 200.0, y * 200.0, z * 200.0];
                let before = view.project(point);
                let mouse = preset == Preset::Mouse;
                let (orbit, pan, zoom) = navigate(
                    preset,
                    delta,
                    (!mouse, false, mouse),
                    mouse,
                    false,
                    camera.distance(),
                    true,
                    0.0,
                    1.0,
                    KEYS_NONE,
                    false,
                );
                camera.navigate(orbit, pan, zoom).unwrap();
                let after = camera.view(1.0).unwrap().project(point);
                let movement = egui::vec2(after[0] - before[0], before[1] - after[1]);
                assert!(
                    movement.dot(delta) > 0.0,
                    "{preset:?}: pointer {delta:?}, projected motion {movement:?}"
                );
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
            (orbit[0] + 0.08).abs() < 1e-6 && (orbit[1] + 0.16).abs() < 1e-6,
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
            (orbit[0] + 0.08).abs() < 1e-6 && (orbit[1] + 0.16).abs() < 1e-6,
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
