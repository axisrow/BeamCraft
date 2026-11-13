// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V12 model views in the window: the session sidecar, the view-properties form, the browser
//! state and the panes of the work area. A view is display state: the book never touches the
//! loaded snapshot, and the model is never written.

use crate::camera::Camera;
use beamcraft_engine::presets::{PresetBook, ViewPreset as PresetBookEntry};
use beamcraft_engine::views::{
    AxisPick, ModelView, PartSide, Projection, ViewBook, WorkArea, validate_folder,
};
use beamcraft_engine::work_plane::WorkPlane;
use beamcraft_engine::{ElementKind, KindFilter, RenderMode};
use beamcraft_json::Limits;
use beamcraft_model::id::Uuid7;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The session sidecar of one store: a sibling `<store>.views.usda` file — the W01 views
/// layer — never inside the store itself. A legacy `<store>.views.json` sidecar still loads.
pub fn session_path(store: &Path) -> PathBuf {
    let mut name = store.as_os_str().to_os_string();
    name.push(".views.usda");
    PathBuf::from(name)
}

/// The legacy JSON sidecar of one store; it is read when no layer exists and is never written.
fn legacy_path(store: &Path) -> PathBuf {
    let mut name = store.as_os_str().to_os_string();
    name.push(".views.json");
    PathBuf::from(name)
}

/// Loads the book for a store. A missing file is an empty book; anything unreadable is an
/// error note, never a reason to refuse the model.
pub fn load(store: &Path) -> Result<ViewBook, String> {
    let bytes = match std::fs::read(session_path(store)) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // A legacy JSON sidecar still carries the views of a store saved before the layer.
            return match std::fs::read(legacy_path(store)) {
                Ok(bytes) => load_bytes(&bytes),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ViewBook::new()),
                Err(e) => Err(format!("the saved views could not be read: {}", e.kind())),
            };
        }
        Err(e) => return Err(format!("the saved views could not be read: {}", e.kind())),
    };
    load_bytes(&bytes)
}

fn load_bytes(bytes: &[u8]) -> Result<ViewBook, String> {
    if bytes.is_empty() {
        return Ok(ViewBook::new());
    }
    if bytes.len() > 1024 * 1024 {
        return Err("the saved views file exceeds 1 MiB".to_owned());
    }
    if session::views_layer::looks_like_layer(bytes) {
        return session::views_layer::book_from_layer(bytes)
            .map_err(|e| format!("the saved views were refused: {}", e.message));
    }
    let json = beamcraft_json::decode(bytes, &Limits::DEFAULT)
        .map_err(|_| "the saved views file is not valid JSON".to_owned())?;
    beamcraft_host::views::book_from_json(&json)
        .map_err(|e| format!("the saved views were refused: {e}"))
}

/// Persists the book through a sibling temporary file, so a failed write keeps the old content.
/// The file also serves the CLI headlessly (`docs/design/v12-named-views.md`), so it may have
/// moved on since the window read it: the disk is read back and merged before the write —
/// what the window never touched keeps the version another process wrote, what it deleted
/// stays deleted, and what it created or changed wins, field by field where the other process
/// changed the same view (issue 569 item 3, the 584 recheck on master). The merged book
/// comes back, so the caller can take it as what the file now holds. An unreadable file
/// refuses the write instead of clobbering it. Temporary views are not written. A store that
/// is not a directory (no model is open there) has nothing to keep the views beside, so
/// nothing is written.
pub fn persist(store: &Path, book: &ViewBook, baseline: &ViewBook) -> Result<ViewBook, String> {
    persist_merging(store, book, baseline).map(|(merged, _)| merged)
}

/// [`persist`], also naming the views the window changed that another process changed too
/// since the window read the file: the merge kept the other process's fields in those, and the
/// window says so instead of changing a view under the user's eyes without a word.
pub fn persist_merging(
    store: &Path,
    book: &ViewBook,
    baseline: &ViewBook,
) -> Result<(ViewBook, Vec<String>), String> {
    if !store.is_dir() {
        return Ok((book.clone(), Vec::new()));
    }
    let path = session_path(store);
    let disk = match std::fs::read(&path) {
        Ok(bytes) => Some(load_bytes(&bytes)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("the saved views could not be read: {}", e.kind())),
    };
    let (merged, shared) = match disk {
        None => (book.clone(), Vec::new()),
        Some(disk) => (
            merge(book, baseline, &disk)?,
            changed_on_both_sides(book, baseline, &disk),
        ),
    };
    let bytes = session::views_layer::layer_bytes(&merged)
        .map_err(|e| format!("the saved views were refused: {}", e.message))?;
    if bytes.len() > 1024 * 1024 {
        return Err("the saved views exceed the file limit".to_owned());
    }
    let temp = path.with_extension("usda.tmp");
    std::fs::write(&temp, &bytes)
        .and_then(|()| std::fs::rename(&temp, &path))
        .map_err(|e| format!("the saved views could not be written: {}", e.kind()))?;
    Ok((merged, shared))
}

/// The names of the views both the window and another process changed since the window last
/// read the file: the window's copy differs from what it read, and so does the file's, in a
/// field other than the plane (the layer keeps the plane as a rotation, so a plane written by
/// the window itself reads back a few bits off).
fn changed_on_both_sides(book: &ViewBook, baseline: &ViewBook, disk: &ViewBook) -> Vec<String> {
    book.views()
        .iter()
        .filter_map(|view| {
            let base = baseline.get(&view.id)?;
            let mut theirs = disk.get(&view.id)?.clone();
            theirs.plane = base.plane;
            (view != base && theirs != *base).then(|| view.name.clone())
        })
        .collect()
}

/// Merges the window's book with what the file holds, over what the window last read or
/// wrote. Views and filters the window changed or created win; ones it never touched keep
/// the disk's version, so an edit elsewhere cannot revert what another process wrote, and
/// ones another process dropped stay dropped; ones the window deleted stay deleted, and ones
/// only the disk has are taken in. Temporary views are session state the file never holds,
/// so they always stay. Folders have no versions: the merge keeps everything except a folder
/// the window deleted and one it never saw.
fn merge(book: &ViewBook, baseline: &ViewBook, disk: &ViewBook) -> Result<ViewBook, String> {
    let mut views = Vec::new();
    for view in disk.views() {
        if let Some(merged) = merge_view(view.id, book, baseline, disk) {
            views.push(merged);
        }
    }
    for view in book.views() {
        if disk.get(&view.id).is_none() {
            // A temporary view lives in this session only — the file never holds it, so the
            // merge has nothing to judge: the session's own copy always stays. A saved view
            // that meanwhile took its name makes `from_all` refuse the merge, and the persist
            // reports that instead of dropping the view silently.
            if view.temporary {
                views.push(view.clone());
            } else if let Some(merged) = merge_view(view.id, book, baseline, disk) {
                views.push(merged);
            }
        }
    }
    let mut filters = Vec::new();
    for filter in disk.filters() {
        if let Some(merged) = merge_filter(&filter.name, book, baseline, disk) {
            filters.push(merged.clone());
        }
    }
    for filter in book.filters() {
        if disk.filter(&filter.name).is_none()
            && let Some(merged) = merge_filter(&filter.name, book, baseline, disk)
        {
            filters.push(merged.clone());
        }
    }
    let mut folders = Vec::new();
    for folder in disk.folders().chain(book.folders()) {
        let dropped =
            baseline.folders().any(|f| f == folder) && !book.folders().any(|f| f == folder);
        if !dropped && !folders.iter().any(|f| f == folder) {
            folders.push(folder.to_owned());
        }
    }
    ViewBook::from_all(views, folders, filters).map_err(|e| e.to_string())
}

/// The merged version of one view by identity: the window's edit when it has one (laid over
/// the disk's version field by field), the disk's view when the window never touched it, gone
/// when the window or the disk removed it.
fn merge_view(
    id: Uuid7,
    book: &ViewBook,
    baseline: &ViewBook,
    disk: &ViewBook,
) -> Option<ModelView> {
    match (book.get(&id), baseline.get(&id), disk.get(&id)) {
        // Changed in the window and still on disk: the window's changed fields over the disk's
        // version, so what another process changed in the same view meanwhile survives.
        (Some(view), Some(kept), Some(theirs)) if view != kept => {
            Some(merge_fields(view, kept, theirs))
        }
        // Changed in the window and dropped from the disk: the window's version comes back.
        (Some(view), Some(kept), None) if view != kept => Some(view.clone()),
        (Some(view), None, _) => Some(view.clone()),
        // Untouched by the window: the disk's version, or its removal, stands.
        (Some(_), Some(_), Some(view)) => Some(view.clone()),
        (Some(_), Some(_), None) => None,
        // Deleted in the window.
        (None, Some(_), _) => None,
        // Only the disk has it: written by another process after the window read.
        (None, None, Some(view)) => Some(view.clone()),
        (None, None, None) => None,
    }
}

/// One view the window changed since it read `base`, while another process wrote `theirs`:
/// `theirs` with every field the window changed (`ours` against `base`) laid over it; a field
/// both changed keeps the window's value. An Apply that only ticked Favourite therefore keeps
/// the render mode and colour the CLI set meanwhile (the 584 recheck on master: the window's
/// whole stale copy replaced them). Every field is named, so a field added to the view has to
/// be merged here as well.
fn merge_fields(ours: &ModelView, base: &ModelView, theirs: &ModelView) -> ModelView {
    fn carry<T: PartialEq + Clone>(slot: &mut T, ours: &T, base: &T) {
        if ours != base {
            slot.clone_from(ours);
        }
    }
    let ModelView {
        id: _,
        name,
        plane,
        projection,
        span_mm,
        depth,
        render_mode,
        grids_visible,
        folder,
        favorite,
        temporary,
        is_default,
        hidden_kinds,
        work_area,
        rotation_rad,
        color,
        opacity,
        overrides,
        representation,
        colour_set,
        provenance,
    } = ours;
    let mut merged = theirs.clone();
    carry(&mut merged.name, name, &base.name);
    carry(&mut merged.plane, plane, &base.plane);
    carry(&mut merged.projection, projection, &base.projection);
    carry(&mut merged.span_mm, span_mm, &base.span_mm);
    carry(&mut merged.depth, depth, &base.depth);
    carry(&mut merged.render_mode, render_mode, &base.render_mode);
    carry(
        &mut merged.grids_visible,
        grids_visible,
        &base.grids_visible,
    );
    carry(&mut merged.folder, folder, &base.folder);
    carry(&mut merged.favorite, favorite, &base.favorite);
    carry(&mut merged.temporary, temporary, &base.temporary);
    carry(&mut merged.is_default, is_default, &base.is_default);
    carry(&mut merged.hidden_kinds, hidden_kinds, &base.hidden_kinds);
    carry(&mut merged.work_area, work_area, &base.work_area);
    carry(&mut merged.rotation_rad, rotation_rad, &base.rotation_rad);
    carry(&mut merged.color, color, &base.color);
    carry(&mut merged.opacity, opacity, &base.opacity);
    carry(&mut merged.overrides, overrides, &base.overrides);
    carry(
        &mut merged.representation,
        representation,
        &base.representation,
    );
    carry(&mut merged.colour_set, colour_set, &base.colour_set);
    carry(&mut merged.provenance, provenance, &base.provenance);
    merged
}

/// [`merge_view`] for a named filter.
fn merge_filter<'a>(
    name: &str,
    book: &'a ViewBook,
    baseline: &'a ViewBook,
    disk: &'a ViewBook,
) -> Option<&'a beamcraft_engine::views::ViewFilter> {
    match (book.filter(name), baseline.filter(name), disk.filter(name)) {
        (Some(view), Some(kept), _) if view != kept => Some(view),
        (Some(view), None, _) => Some(view),
        (Some(_), Some(_), Some(view)) => Some(view),
        (Some(_), Some(_), None) => None,
        (None, Some(_), _) => None,
        (None, None, Some(view)) => Some(view),
        (None, None, None) => None,
    }
}

/// The file the user's discipline view presets (V07) are kept in, inside the configuration
/// directory: personal configuration, like the colour sets, never part of a model.
pub const PRESETS_FILE: &str = "view-presets.json";

/// Reads the user's view presets from `config_dir` (the platform default when `None`). A missing
/// file is an empty book; an unreadable one is an empty book and a note.
pub fn load_presets(config_dir: Option<&Path>) -> (PresetBook, Option<String>) {
    let Some(dir) =
        config_dir.map_or_else(crate::shortcuts::default_dir, |d| Some(d.to_path_buf()))
    else {
        return (PresetBook::new(), None);
    };
    let bytes = match std::fs::read(dir.join(PRESETS_FILE)) {
        Ok(bytes) if bytes.len() <= beamcraft_host::presets::MAX_BYTES => bytes,
        Ok(_) => {
            return (
                PresetBook::new(),
                Some("the view presets file is larger than 64 KiB".into()),
            );
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (PresetBook::new(), None),
        Err(e) => {
            return (
                PresetBook::new(),
                Some(format!("the view presets could not be read: {}", e.kind())),
            );
        }
    };
    let book = beamcraft_json::decode(&bytes, &Limits::DEFAULT)
        .map_err(|_| "the view presets file is not valid JSON".to_owned())
        .and_then(|json| {
            beamcraft_host::presets::book_from_json(&json)
                .map_err(|e| format!("the view presets were refused: {e}"))
        });
    match book {
        Ok(book) => (book, None),
        Err(note) => (PresetBook::new(), Some(note)),
    }
}

/// Writes the user's view presets into `config_dir` (the platform default when `None`) through
/// a sibling temporary file, so a failed write keeps the old content.
pub fn persist_presets(config_dir: Option<&Path>, book: &PresetBook) -> Result<(), String> {
    let Some(dir) =
        config_dir.map_or_else(crate::shortcuts::default_dir, |d| Some(d.to_path_buf()))
    else {
        return Err("there is no configuration directory for the view presets".into());
    };
    let bytes = beamcraft_host::presets::book_to_json(book)
        .and_then(|json| {
            json.encode(&Limits::DEFAULT).map_err(|_| {
                beamcraft_model::command::CommandError::new(
                    beamcraft_model::error::ErrorCode::InvalidArgument,
                    "presets",
                    "the view presets do not encode",
                )
            })
        })
        .map_err(|e| format!("the view presets were refused: {e}"))?;
    if bytes.len() > beamcraft_host::presets::MAX_BYTES {
        return Err("the view presets exceed 64 KiB".into());
    }
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("the view presets could not be written: {}", e.kind()))?;
    let path = dir.join(PRESETS_FILE);
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, &bytes)
        .and_then(|()| std::fs::rename(&temp, &path))
        .map_err(|e| format!("the view presets could not be written: {}", e.kind()))
}

/// How a new view gets its plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Construction {
    /// The model XY plane at an offset: a plan.
    #[default]
    PlanXy,
    /// The model XZ plane at an offset: an elevation.
    ElevationXz,
    /// The model YZ plane at an offset: an elevation.
    ElevationYz,
    /// A vertical section through two picked points, across the input plane.
    TwoPoints,
    /// A plane through three picked points.
    ThreePoints,
    /// The current work plane.
    WorkPlane,
    /// Along grid axes: one axis or a batch per family.
    GridAxes,
    /// The plan of a model level, or a batch of them.
    Level,
    /// The face picked in the model.
    Surface,
    /// The face picked in the model, its X along two picked points on it.
    SurfaceEdge,
    /// A part seen from one side, or its four views.
    Part,
    /// An assembly seen from one side, or its four views.
    Assembly,
    /// The camera as it is now.
    CurrentCamera,
}

impl Construction {
    /// Every construction, in menu order, with its label.
    pub const ALL: [(Self, &'static str); 13] = [
        (Self::PlanXy, "Plan (XY plane)"),
        (Self::ElevationXz, "Elevation (XZ plane)"),
        (Self::ElevationYz, "Elevation (YZ plane)"),
        (Self::TwoPoints, "Section by two points"),
        (Self::ThreePoints, "View by three points"),
        (Self::WorkPlane, "On the work plane"),
        (Self::GridAxes, "Along grid axes"),
        (Self::Level, "Level plans"),
        (Self::Surface, "On a part face"),
        (Self::SurfaceEdge, "On a face, along an edge"),
        (Self::Part, "Part views"),
        (Self::Assembly, "Assembly views"),
        (Self::CurrentCamera, "From the current camera"),
    ];

    /// The menu label.
    pub fn label(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(c, _)| *c == self)
            .map_or("View", |(_, label)| label)
    }

    /// The points picked in the model before the view can be built: the surface counts as one.
    pub fn picks(self) -> usize {
        match self {
            Self::TwoPoints => 2,
            Self::ThreePoints => 3,
            Self::Surface => 1,
            Self::SurfaceEdge => 3,
            _ => 0,
        }
    }

    /// The prompt for the next pick, given how many are in.
    pub fn prompt(self, done: usize) -> &'static str {
        match (self, done) {
            (Self::TwoPoints, 0) => "Pick the first point of the section line",
            (Self::TwoPoints, _) => "Pick the second point of the section line",
            (Self::ThreePoints, 0) => "Pick the origin of the view plane",
            (Self::ThreePoints, 1) => "Pick a point along the view's X axis",
            (Self::ThreePoints, _) => "Pick a point on the view's upper side",
            (Self::Surface | Self::SurfaceEdge, 0) => "Pick a planar face of a part",
            (Self::SurfaceEdge, 1) => "Pick the start of the edge on that face",
            (Self::SurfaceEdge, _) => "Pick the end of the edge on that face",
            _ => "",
        }
    }
}

/// The browser tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BrowserTab {
    /// Model views.
    #[default]
    Views,
    /// Drawings (listed elsewhere for now).
    Drawings,
    /// Schedules (listed elsewhere for now).
    Schedules,
}

/// The six directional cameras: they turn the camera, never the input plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Looking down.
    Top,
    /// Looking up.
    Bottom,
    /// Looking along +Y.
    Front,
    /// Looking along -Y.
    Back,
    /// Looking along +X.
    Left,
    /// Looking along -X.
    Right,
}

impl Direction {
    /// Every direction with its label.
    pub const ALL: [(Self, &'static str); 6] = [
        (Self::Top, "Top"),
        (Self::Bottom, "Bottom"),
        (Self::Front, "Front"),
        (Self::Back, "Back"),
        (Self::Left, "Left"),
        (Self::Right, "Right"),
    ];

    /// The plane the camera looks at: its normal points towards the viewer.
    pub fn plane(self, origin: [f64; 3]) -> Result<WorkPlane, String> {
        let (x, y) = match self {
            Self::Top => ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            Self::Bottom => ([1.0, 0.0, 0.0], [0.0, -1.0, 0.0]),
            Self::Front => ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            Self::Back => ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            Self::Left => ([0.0, -1.0, 0.0], [0.0, 0.0, 1.0]),
            Self::Right => ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        };
        WorkPlane::new(origin, x, y).map_err(|e| e.to_string())
    }
}

/// A grid batch: one axis, or each family on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AxisBatch {
    /// One chosen axis.
    #[default]
    Single,
    /// The U and V families, each picked separately.
    Families,
}

/// What the side combo of part and assembly views offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SideChoice {
    /// The four views: front, top, end and 3D.
    #[default]
    Four,
    /// One side.
    One(PartSide),
}

/// The text a bounded field takes.
pub const FIELD: usize = 128;

/// The view-properties form: text until an explicit action, so typing never moves the camera.
#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    /// The view being edited; `None` for a new view.
    pub id: Option<Uuid7>,
    /// Its plane, kept for an existing view.
    pub plane: WorkPlane,
    /// Name, or the name prefix of a batch; empty makes the view temporary.
    pub name: String,
    /// Browser folder.
    pub folder: String,
    /// Quick access mark.
    pub favorite: bool,
    /// The model's default view.
    pub default: bool,
    /// How a new view gets its plane.
    pub construction: Construction,
    /// Offset of a coordinate plane, mm.
    pub offset: String,
    /// Index into the displayed grids.
    pub grid: usize,
    /// One axis or families.
    pub axis_batch: AxisBatch,
    /// The chosen axis label.
    pub axis: String,
    /// What each family takes in a batch.
    pub families: [AxisPick; 2],
    /// Name prefix of each family.
    pub prefixes: [String; 2],
    /// One level (`None`) or a batch.
    pub levels: Option<AxisPick>,
    /// The chosen level name.
    pub level: String,
    /// Part and assembly side.
    pub side: SideChoice,
    /// Projection.
    pub projection: Projection,
    /// Visible height, mm.
    pub span: String,
    /// Whether the depth is limited.
    pub depth_on: bool,
    /// Depth towards the viewer and behind the plane, mm.
    pub depth: [String; 2],
    /// Camera rotation about the view Z and X, degrees.
    pub rotation: [String; 2],
    /// Render mode.
    pub mode: RenderMode,
    /// Grid guides shown.
    pub grids: bool,
    /// Hidden kinds.
    pub hidden: KindFilter,
    /// The work area, when set.
    pub area: Option<WorkArea>,
    /// Whether the colour is overridden.
    pub color_on: bool,
    /// The override colour, 0..1.
    pub color: [f32; 3],
    /// Opacity, 0..1.
    pub opacity: f32,
    /// The filter overrides, in order: the first whose filter matches decides.
    pub overrides: Vec<beamcraft_engine::views::FilterOverride>,
    /// Whether the picks of a pick-based construction are all in.
    pub picked: bool,
}

impl Default for Form {
    fn default() -> Self {
        Self {
            id: None,
            plane: WorkPlane::default(),
            name: String::new(),
            folder: String::new(),
            favorite: false,
            default: false,
            construction: Construction::default(),
            offset: "0".into(),
            grid: 0,
            axis_batch: AxisBatch::default(),
            axis: String::new(),
            families: [AxisPick::All, AxisPick::All],
            prefixes: ["Grid".into(), "Grid".into()],
            levels: None,
            level: String::new(),
            side: SideChoice::default(),
            projection: Projection::Orthographic,
            span: format!("{}", beamcraft_engine::views::DEFAULT_SPAN_MM),
            depth_on: false,
            depth: ["1000".into(), "1000".into()],
            rotation: ["0".into(), "0".into()],
            mode: RenderMode::Shaded,
            grids: true,
            hidden: KindFilter::SHOW_ALL,
            area: None,
            color_on: false,
            color: [0.65, 0.75, 0.8],
            opacity: 1.0,
            overrides: Vec::new(),
            picked: false,
        }
    }
}

fn number(text: &str, what: &str) -> Result<f64, String> {
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("{what} needs a number"))
}

impl Form {
    /// The form of an existing view.
    pub fn from_view(view: &ModelView) -> Self {
        let mut form = Self {
            id: Some(view.id),
            plane: view.plane,
            name: String::new(),
            folder: view.folder.clone(),
            favorite: view.favorite,
            default: view.is_default,
            ..Self::default()
        };
        // A temporary view is the one without a name.
        if !view.temporary {
            form.name.clone_from(&view.name);
        }
        form.copy_display(view);
        form
    }

    /// Takes the display properties of `view`, nothing else.
    pub fn copy_display(&mut self, view: &ModelView) {
        self.projection = view.projection;
        self.span = format!("{}", view.span_mm.round().max(1.0));
        self.depth_on = view.depth.is_some();
        if let Some([above, below]) = view.depth {
            self.depth = [format!("{above}"), format!("{below}")];
        }
        self.rotation = view.rotation_rad.map(degree_text);
        self.mode = view.render_mode;
        self.grids = view.grids_visible;
        self.hidden = view.hidden_kinds;
        self.area = view.work_area;
        self.color_on = view.color.is_some();
        if let Some(color) = view.color {
            self.color = color.map(|c| c as f32);
        }
        self.opacity = view.opacity as f32;
        self.overrides.clone_from(&view.overrides);
    }

    /// Fills the display fields a discipline preset carries (the depth slab, the grid toggle,
    /// the hidden kinds and the filter overrides), as a part preset fills a new part's form.
    pub fn copy_preset(&mut self, preset: &PresetBookEntry) {
        self.depth_on = preset.depth.is_some();
        if let Some([above, below]) = preset.depth {
            self.depth = [format!("{above}"), format!("{below}")];
        }
        self.grids = preset.grids_visible;
        self.hidden = preset.hidden_kinds;
        self.overrides.clone_from(&preset.overrides);
    }

    /// Writes the identity-free properties of the form into `view`: name, folder, marks and
    /// every display property. The plane is the caller's. An empty name makes the view
    /// temporary and leaves its book name as it is.
    pub fn apply(&self, view: &mut ModelView) -> Result<(), String> {
        let name = self.name.trim();
        view.temporary = name.is_empty();
        if view.temporary {
            if view.name.is_empty() {
                return Err("give the view a name".into());
            }
        } else {
            name.clone_into(&mut view.name);
        }
        self.folder.trim().clone_into(&mut view.folder);
        validate_folder(&view.folder).map_err(|e| e.to_string())?;
        view.favorite = self.favorite;
        view.is_default = self.default;
        if view.is_default && view.temporary {
            return Err("the default view needs a name".into());
        }
        view.projection = self.projection;
        view.span_mm = number(&self.span, "the visible height")?;
        view.depth = if self.depth_on {
            Some([
                number(&self.depth[0], "the depth towards the viewer")?,
                number(&self.depth[1], "the depth behind the plane")?,
            ])
        } else {
            None
        };
        view.rotation_rad = [
            number(&self.rotation[0], "the rotation about Z")?.to_radians(),
            number(&self.rotation[1], "the rotation about X")?.to_radians(),
        ];
        view.render_mode = self.mode;
        view.grids_visible = self.grids;
        view.hidden_kinds = self.hidden;
        view.work_area = self.area;
        view.color = self.color_on.then(|| self.color.map(f64::from));
        view.opacity = f64::from(self.opacity).clamp(0.0, 1.0);
        view.overrides.clone_from(&self.overrides);
        view.validate().map_err(|e| e.to_string())
    }

    /// [`Self::apply`] for an existing view: a value the form still shows exactly as it showed
    /// it for `view` (the visible height, each rotation, the colour, the opacity) keeps the
    /// view's own value, which the form's text and single-precision fields only approximate.
    /// An Apply then changes nothing the user did not touch, and the merge with another
    /// process's copy of the view cannot take a rounding for an edit.
    pub fn apply_edits(&self, view: &mut ModelView) -> Result<(), String> {
        let before = view.clone();
        let shown = Self::from_view(&before);
        self.apply(view)?;
        if self.span == shown.span {
            view.span_mm = before.span_mm;
        }
        for ((text, was), (value, kept)) in self
            .rotation
            .iter()
            .zip(&shown.rotation)
            .zip(view.rotation_rad.iter_mut().zip(before.rotation_rad))
        {
            if text == was {
                *value = kept;
            }
        }
        if self.color_on == shown.color_on && self.color == shown.color {
            view.color = before.color;
        }
        if self.opacity == shown.opacity {
            view.opacity = before.opacity;
        }
        view.validate().map_err(|e| e.to_string())
    }

    /// The offset of a coordinate-plane view.
    pub fn plane_offset(&self) -> Result<f64, String> {
        number(&self.offset, "the plane offset")
    }

    /// Writes into `target` every display and browser property this form changed from
    /// `baseline` (the form of the view as it was loaded), and nothing else: a multi-selection
    /// edit changes on every selected view what the user changed on one. The name, the plane and
    /// the default mark belong to one view and are never carried; each depth and rotation value
    /// is carried on its own, so a changed depth towards the viewer keeps each view's own depth
    /// behind its plane.
    pub fn carry_changes(&self, baseline: &Self, target: &mut Self) {
        fn carry<T: PartialEq + Clone>(edited: &T, baseline: &T, target: &mut T) {
            if edited != baseline {
                target.clone_from(edited);
            }
        }
        carry(&self.folder, &baseline.folder, &mut target.folder);
        carry(&self.favorite, &baseline.favorite, &mut target.favorite);
        carry(
            &self.projection,
            &baseline.projection,
            &mut target.projection,
        );
        carry(&self.span, &baseline.span, &mut target.span);
        carry(&self.depth_on, &baseline.depth_on, &mut target.depth_on);
        for ((edited, baseline), target) in self
            .depth
            .iter()
            .zip(&baseline.depth)
            .zip(&mut target.depth)
        {
            carry(edited, baseline, target);
        }
        for ((edited, baseline), target) in self
            .rotation
            .iter()
            .zip(&baseline.rotation)
            .zip(&mut target.rotation)
        {
            carry(edited, baseline, target);
        }
        carry(&self.mode, &baseline.mode, &mut target.mode);
        carry(&self.grids, &baseline.grids, &mut target.grids);
        carry(&self.hidden, &baseline.hidden, &mut target.hidden);
        carry(&self.area, &baseline.area, &mut target.area);
        carry(&self.color_on, &baseline.color_on, &mut target.color_on);
        carry(&self.color, &baseline.color, &mut target.color);
        carry(&self.opacity, &baseline.opacity, &mut target.opacity);
        carry(&self.overrides, &baseline.overrides, &mut target.overrides);
    }
}

/// The degree text of a rotation held in radians: the coarsest rounded degree value that
/// turns back into the very same radians. Degrees round-tripped through radians otherwise
/// never read back exactly, so 30 came back as "29.999999999999996" and kept the form dirty
/// after Apply (issue 569 item 20). A rotation no rounding reproduces keeps the full text.
fn degree_text(rad: f64) -> String {
    let deg = rad.to_degrees();
    for digits in [0_u32, 3, 6, 9, 12] {
        let scale = 10_f64.powi(digits as i32);
        let rounded = (deg * scale).round() / scale;
        if rounded.to_radians() == rad {
            return format!("{rounded}");
        }
    }
    format!("{deg}")
}

/// A view-construction pick in progress: points picked in the model, and the face of a surface
/// view once picked.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Picking {
    /// What is being built.
    pub construction: Construction,
    /// The picked points, model millimetres.
    pub points: Vec<[f64; 3]>,
    /// The face plane of a surface view.
    pub face: Option<WorkPlane>,
}

impl Picking {
    /// How many picks are in.
    pub fn done(&self) -> usize {
        self.points.len() + usize::from(self.face.is_some())
    }

    /// Removes the last pick; false when there was none.
    pub fn undo(&mut self) -> bool {
        if self.points.pop().is_some() {
            return true;
        }
        self.face.take().is_some()
    }
}

/// How a view's plane lies, as the browser groups it. Every view can be looked at in 3D (the
/// 3D/plane toggle orbits any of them), so "3D" is a way of looking, not a kind: the kind is
/// the orientation of the plane the view is built on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum ViewKind {
    /// A horizontal plane: the view looks straight down (or up).
    #[default]
    Plan,
    /// A vertical plane: an elevation, or a section when its depth is limited.
    Vertical,
    /// A plane neither horizontal nor vertical.
    Inclined,
}

impl ViewKind {
    /// The kinds in browser order.
    pub const ALL: [Self; 3] = [Self::Plan, Self::Vertical, Self::Inclined];

    /// The group title.
    pub fn label(self) -> &'static str {
        match self {
            Self::Plan => "Plans",
            Self::Vertical => "Elevations & sections",
            Self::Inclined => "Inclined",
        }
    }

    /// The kind with this group title.
    pub fn of_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.label() == label)
    }

    /// The icon of the kind.
    pub fn icon(self) -> crate::theme::Icon {
        match self {
            Self::Plan => crate::theme::Icon::PlanView,
            Self::Vertical => crate::theme::Icon::ElevationView,
            Self::Inclined => crate::theme::Icon::InclinedView,
        }
    }
}

/// The kind of `view`: from its projection, the direction its camera looks along and whether
/// its depth is limited.
pub fn view_kind(view: &ModelView) -> ViewKind {
    // The plane the view is built on decides, not the camera turned about it: a plan stays a
    // plan while it is orbited in 3D.
    let up = view.plane.normal()[2].abs();
    if up > 0.999 {
        ViewKind::Plan
    } else if up < 0.001 {
        ViewKind::Vertical
    } else {
        ViewKind::Inclined
    }
}

/// One folder of the browser tree: its name, the stored folder path its views carry, its
/// subfolders and its views.
#[derive(Debug, Clone, PartialEq)]
pub struct FolderNode {
    /// The last path segment, as listed.
    pub name: String,
    /// The folder path a view dropped here gets (empty for a kind's own top level).
    pub stored: String,
    /// Subfolders by name.
    pub folders: Vec<FolderNode>,
    /// The views directly in this folder, the default first, then by name.
    pub views: Vec<Uuid7>,
}

impl FolderNode {
    fn new(name: &str, stored: String) -> Self {
        Self {
            name: name.to_owned(),
            stored,
            folders: Vec::new(),
            views: Vec::new(),
        }
    }

    /// Makes the folder at the shown `path` under this one (with its ancestors; each new node's
    /// stored path is `prefix` and the path so far) and lists `view` in it.
    fn place(&mut self, path: &[&str], prefix: &str, view: Option<Uuid7>) {
        let Some((first, rest)) = path.split_first() else {
            self.views.extend(view);
            return;
        };
        let at = match self.folders.iter().position(|f| f.name == *first) {
            Some(at) => at,
            None => {
                let stored = if self.stored.is_empty() {
                    if prefix.is_empty() {
                        (*first).to_owned()
                    } else {
                        format!("{prefix}/{first}")
                    }
                } else {
                    format!("{}/{first}", self.stored)
                };
                self.folders.push(Self::new(first, stored));
                self.folders.len() - 1
            }
        };
        if let Some(node) = self.folders.get_mut(at) {
            node.place(rest, prefix, view);
        }
    }

    /// Every view in this folder and below, in listed order.
    pub fn all_views(&self) -> Vec<Uuid7> {
        let mut out = Vec::new();
        for folder in &self.folders {
            out.extend(folder.all_views());
        }
        out.extend(self.views.iter().copied());
        out
    }

    fn sort(&mut self, book: &ViewBook) {
        self.folders.sort_by_key(|f| f.name.to_lowercase());
        for folder in &mut self.folders {
            folder.sort(book);
        }
        self.views.sort_by_key(|id| {
            book.get(id).map_or((true, String::new()), |v| {
                (!v.is_default, v.name.to_lowercase())
            })
        });
    }
}

/// The stored path a new folder `name` gets inside `parent` (a stored path; empty for the top
/// level of the browser).
pub fn new_folder_path(parent: &str, name: &str) -> String {
    let name = name.trim();
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    }
}

/// What the view panel shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject {
    /// The free 3D view of the pane with this key (`None` when no pane is open): its display
    /// and "Save as view".
    FreeView(Option<u64>),
    /// A new view being defined; Apply creates it.
    NewView,
    /// A saved view; Apply updates it.
    SavedView(Uuid7),
}

impl Default for Subject {
    fn default() -> Self {
        Self::FreeView(None)
    }
}

impl Subject {
    /// The inspector title.
    pub fn title(self) -> &'static str {
        match self {
            Self::FreeView(_) => "3D view",
            Self::NewView => "New view",
            Self::SavedView(_) => "View properties",
        }
    }
}

/// The views of one opened model in the window.
#[derive(Default)]
pub struct Views {
    /// The book: saved and temporary views and the folders.
    pub book: ViewBook,
    /// What this window last read from the views file or wrote to it: the baseline a persist
    /// merges against, so an edit here cannot revert what the CLI wrote to the same file
    /// (issue 569 item 3).
    pub baseline: ViewBook,
    /// The user's discipline view presets (V07), kept in the configuration directory.
    pub presets: PresetBook,
    /// Why the last action failed, or what it did.
    pub note: Option<Result<String, String>>,
    /// The panes of the work area in z-order: the last one is active.
    pub panes: Vec<crate::panes::ViewPane>,
    /// The last key handed to a pane.
    pub pane_serial: u64,
    /// The session camera of each opened view, so returning to a view keeps its navigation.
    pub cameras: BTreeMap<Uuid7, Camera>,
    /// Views opened in this session, in opening order (Ctrl+Tab cycles them).
    pub open: Vec<Uuid7>,
    /// The properties form.
    pub form: Form,
    /// What the view panel shows while the inspector is on the view bench.
    pub subject: Subject,
    /// A pick in progress.
    pub picking: Option<Picking>,
    /// The last work-area rect, in points; `None` before the first work-area frame.
    pub area: Option<egui::Rect>,
    /// The tab layout of the work area: which pane is a tab of which group, and where the
    /// groups sit.
    pub dock: crate::dock::Dock,
    /// The tabs drawn last frame: pane key, tab rect and close glyph rect.
    pub tabs_drawn: Vec<(u64, egui::Rect, egui::Rect)>,
    /// What the filters of the open views match on the head on screen.
    pub matches: crate::view_overlay::FilterMatches,
    /// The overlay each open view with filter overrides draws with: the key it was built for,
    /// the overlay, and the filters that were refused (name and reason).
    pub overlays: BTreeMap<Uuid7, ViewOverlay>,
    /// The pane a primary press just focused, until that button is released: its click only
    /// focuses unless it may select.
    pub focus_press: Option<u64>,
    /// Browser tab.
    pub tab: BrowserTab,
    /// Browser search.
    pub search: String,
    /// Show only favourites.
    pub favorites_only: bool,
    /// Show only quick access: favourites, temporary and open views.
    pub quick_access: bool,
    /// Selected views in the browser.
    pub selected: BTreeSet<Uuid7>,
    /// The anchor of a Shift range.
    pub anchor: Option<Uuid7>,
    /// Destination of a paste or a new folder.
    pub destination: String,
    /// Whether the new-folder field is shown.
    pub new_folder: bool,
    /// The folder being renamed in place: its stored path and the name being typed.
    pub folder_rename: Option<(String, String)>,
    /// The name a "Save as view preset" gets, while its field is shown.
    pub preset_name: Option<String>,
    /// The filter editor: the filter being edited (a working copy) and the name it was saved
    /// under (`None` for a new one).
    pub filter_edit: Option<(beamcraft_engine::views::ViewFilter, Option<String>)>,
    /// Where the new folder goes: the stored path of its parent folder, empty for the top level.
    pub folder_parent: String,
    /// A delete waiting for confirmation.
    pub confirm_delete: Option<Uuid7>,
    /// A store-backed construction running in the CLI.
    pub job: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    /// The hidden kinds the loaded scene was built with.
    pub shown: KindFilter,
    /// Whether the browser shows previews.
    pub previews_on: bool,
    /// The rendered previews: the key they were drawn for (snapshot and definition) and the
    /// offscreen target, at most [`MAX_PREVIEWS`].
    pub previews: BTreeMap<Uuid7, (u64, crate::panes::RenderTarget)>,
}

/// The overlay one view draws its filter overrides with.
#[derive(Clone)]
pub struct ViewOverlay {
    /// What it was built from (`crate::view_overlay::overlay_key`).
    pub key: u64,
    /// The colours and hidden elements.
    pub overlay: std::sync::Arc<crate::colour_sets::Overlay>,
    /// Whether every override's filter has been evaluated.
    pub complete: bool,
    /// The filters the evaluator refused, by name, with the reason.
    pub refused: Vec<(String, String)>,
}

/// Most views open at once as tabs in the work area.
pub const MAX_WINDOWS: usize = 16;
/// Most previews kept rendered.
pub const MAX_PREVIEWS: usize = 64;
/// Preview size in pixels.
pub const PREVIEW_SIZE: [u32; 2] = [160, 100];

/// The key a preview is drawn for: the snapshot token and every field of the view.
pub fn preview_key(token: &str, view: &ModelView) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    token.hash(&mut hasher);
    format!("{view:?}").hash(&mut hasher);
    hasher.finish()
}

impl Views {
    /// The view the active pane shows; `None` is the free 3D view or no pane at all.
    pub fn active(&self) -> Option<Uuid7> {
        self.panes.last().and_then(|p| p.view)
    }

    /// The active pane: the last in z-order.
    pub fn active_pane(&self) -> Option<&crate::panes::ViewPane> {
        self.panes.last()
    }

    /// The active pane, to change.
    pub fn active_pane_mut(&mut self) -> Option<&mut crate::panes::ViewPane> {
        self.panes.last_mut()
    }

    /// The active view.
    pub fn active_view(&self) -> Option<&ModelView> {
        self.active().and_then(|id| self.book.get(&id))
    }

    /// The hidden kinds of the active view.
    pub fn active_hidden(&self) -> KindFilter {
        self.active_view()
            .map_or(KindFilter::SHOW_ALL, |v| v.hidden_kinds)
    }

    /// The tint the active view draws with.
    pub fn tint(&self) -> [f32; 4] {
        tint_of(self.active_view())
    }

    /// Whether the browser lists `view` under the search and the favourites and quick-access
    /// switches.
    fn listed(&self, view: &ModelView) -> bool {
        let needle = self.search.trim().to_lowercase();
        (!self.favorites_only || view.favorite)
            && (!self.quick_access
                || view.favorite
                || view.temporary
                || self.open.contains(&view.id))
            && format!("{} {}", view.name, view.folder)
                .to_lowercase()
                .contains(&needle)
    }

    /// The default view when the browser lists it: it heads the tree on its own row, above
    /// the kinds, since it is the model's home view rather than one of a kind.
    pub fn browser_home(&self) -> Option<Uuid7> {
        self.book
            .default_view()
            .filter(|view| self.listed(view))
            .map(|view| view.id)
    }

    /// The browser tree: the views in their folders, as Revit's project browser lists views
    /// when it is organised by folder. The kind of a view (plan, elevation or section,
    /// inclined) shows as its icon, not as a group; the default view is not in the tree
    /// ([`Self::browser_home`]). Folders come first, then the views, each by name. Empty
    /// folders are hidden while a search or switch filters the list.
    pub fn browser_tree(&self) -> FolderNode {
        let filtering = !self.search.trim().is_empty() || self.favorites_only || self.quick_access;
        let mut root = FolderNode::new("Views", String::new());
        for view in self.book.views() {
            if view.is_default || !self.listed(view) {
                continue;
            }
            let path: Vec<&str> = view.folder.split('/').filter(|s| !s.is_empty()).collect();
            root.place(&path, "", Some(view.id));
        }
        if !filtering {
            for folder in self.book.folders() {
                let path: Vec<&str> = folder.split('/').filter(|s| !s.is_empty()).collect();
                root.place(&path, "", None);
            }
        }
        root.sort(&self.book);
        root
    }

    /// Applies a browser click: plain selects one, Ctrl toggles, Shift extends from the anchor
    /// over `order` (the listed order). Returns true for a plain click.
    pub fn click(&mut self, id: Uuid7, ctrl: bool, shift: bool, order: &[Uuid7]) -> bool {
        let plain = !ctrl && !shift;
        if shift {
            let from = self.anchor.and_then(|a| order.iter().position(|v| *v == a));
            let to = order.iter().position(|v| *v == id);
            if let (Some(a), Some(b)) = (from, to)
                && let Some(range) = order.get(a.min(b)..=a.max(b))
            {
                self.selected.extend(range.iter().copied());
            }
        } else if ctrl {
            if !self.selected.remove(&id) {
                self.selected.insert(id);
            }
        } else {
            self.selected.clear();
            self.selected.insert(id);
        }
        self.anchor = Some(id);
        plain
    }

    /// The next open view after the active one, for Ctrl+Tab.
    pub fn next_open(&self) -> Option<Uuid7> {
        let open: Vec<Uuid7> = self
            .open
            .iter()
            .copied()
            .filter(|id| self.book.get(id).is_some())
            .collect();
        let at = self
            .active()
            .and_then(|a| open.iter().position(|v| *v == a));
        match at {
            Some(i) => open.get((i + 1) % open.len().max(1)).copied(),
            None => open.first().copied(),
        }
    }

    /// Forgets a view in the session: its camera and the open and selected marks. Its panes are closed by the window.
    pub fn forget(&mut self, id: &Uuid7) {
        self.cameras.remove(id);
        self.open.retain(|v| v != id);
        self.selected.remove(id);
    }
}

/// The shader tint of a view: its colour override and opacity.
pub fn tint_of(view: Option<&ModelView>) -> [f32; 4] {
    match view {
        Some(view) => {
            let [r, g, b] = view.color.map_or([-1.0, 0.0, 0.0], |c| c.map(|v| v as f32));
            [r, g, b, view.opacity as f32]
        }
        None => crate::camera::NO_TINT,
    }
}

/// The display cut of a window: the V15 cut and the view's own depth slab and work area
/// together. Planes add up; two work areas intersect. `None` when nothing is cut.
pub fn display_clip(
    cut: Option<&beamcraft_engine::view_clipping::Clipping>,
    view: Option<&ModelView>,
) -> Option<beamcraft_engine::view_clipping::Clipping> {
    let own = view.and_then(|v| v.clipping().ok());
    let mut planes = Vec::new();
    let mut area: Option<WorkArea> = None;
    for clip in [cut, own.as_ref()].into_iter().flatten() {
        planes.extend(clip.planes.iter().copied());
        area = match (area, clip.area) {
            (Some(a), Some(b)) => {
                let each = |[x, y, z]: [f64; 3], [p, q, r]: [f64; 3], f: fn(f64, f64) -> f64| {
                    [f(x, p), f(y, q), f(z, r)]
                };
                let min = each(a.min, b.min, f64::max);
                let max = each(a.max, b.max, f64::min);
                // Disjoint boxes show nothing: a degenerate box at the first one's corner.
                WorkArea::new(min, max)
                    .ok()
                    .or_else(|| WorkArea::new(a.min, a.min).ok())
            }
            (a, b) => a.or(b),
        };
    }
    let clip = beamcraft_engine::view_clipping::Clipping { planes, area };
    (!clip.planes.is_empty() || clip.area.is_some()).then_some(clip)
}

/// The element kinds the object-visibility list offers: every registered kind.
pub fn listed_kinds() -> impl Iterator<Item = ElementKind> {
    ElementKind::ALL.into_iter()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn id(n: u64) -> Uuid7 {
        Uuid7::parse(&format!("019a7bf0-0000-7000-8000-{n:012}")).unwrap()
    }

    fn view(n: u64, name: &str, folder: &str) -> ModelView {
        let mut v = ModelView::new(id(n), name, WorkPlane::default(), 5000.0);
        v.folder = folder.into();
        v
    }

    #[test]
    fn the_sidecar_round_trips_and_reports_unreadable_files() {
        let dir = std::env::temp_dir().join(format!("beamcraft-views-ui-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join("m.store");
        std::fs::create_dir_all(&store).unwrap();
        assert!(load(&store).unwrap().is_empty());
        assert_eq!(session_path(&store), dir.join("m.store.views.usda"));
        let mut book = ViewBook::new();
        book.insert(view(1, "Plan", "Plans")).unwrap();
        let mut temp = view(2, "Scratch", "");
        temp.temporary = true;
        book.insert(temp).unwrap();
        persist(&store, &book, &ViewBook::new()).unwrap();
        // The sidecar is the views layer: a camera-prim USD document, not JSON.
        let written = std::fs::read(session_path(&store)).unwrap();
        assert!(session::views_layer::looks_like_layer(&written));
        let back = load(&store).unwrap();
        assert_eq!(back.len(), 1, "temporary views are not written");
        assert_eq!(back.get(&id(1)), book.get(&id(1)));
        std::fs::write(session_path(&store), b"{not json").unwrap();
        assert!(load(&store).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_legacy_json_sidecar_still_loads_and_never_preempts_the_layer() {
        let dir =
            std::env::temp_dir().join(format!("beamcraft-views-ui-legacy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join("m.store");
        std::fs::create_dir_all(&store).unwrap();
        let mut book = ViewBook::new();
        book.insert(view(1, "Plan", "Plans")).unwrap();
        std::fs::write(
            legacy_path(&store),
            beamcraft_host::views::book_to_json(&book)
                .unwrap()
                .encode(&Limits::DEFAULT)
                .unwrap(),
        )
        .unwrap();
        let back = load(&store).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back.get(&id(1)), book.get(&id(1)));
        // A layer beside the legacy sidecar wins; the JSON copy is never read again.
        persist(&store, &back, &ViewBook::new()).unwrap();
        std::fs::write(legacy_path(&store), b"{broken").unwrap();
        let layer = load(&store).unwrap();
        assert_eq!(layer.len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn persisting_merges_what_another_process_wrote() {
        let dir =
            std::env::temp_dir().join(format!("beamcraft-views-ui-merge-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join("m.store");
        std::fs::create_dir_all(&store).unwrap();
        let elev = view(1, "Elev", "Plans");
        let baseline =
            ViewBook::from_all(vec![elev.clone()], ["Plans".to_owned()], Vec::new()).unwrap();
        persist(&store, &baseline, &baseline).unwrap();
        // Another process turns Elev wireframe and adds AgentView, writing the file directly.
        let mut updated = elev.clone();
        updated.render_mode = RenderMode::Wireframe;
        let cli = ViewBook::from_all(
            vec![updated, view(2, "AgentView", "")],
            ["Plans".to_owned()],
            Vec::new(),
        )
        .unwrap();
        std::fs::write(
            session_path(&store),
            session::views_layer::layer_bytes(&cli).unwrap(),
        )
        .unwrap();
        // The stale window edits nothing but a folder: the other process's work survives.
        let book = ViewBook::from_all(
            vec![elev.clone()],
            ["Plans".to_owned(), "F".to_owned()],
            Vec::new(),
        )
        .unwrap();
        let merged = persist(&store, &book, &baseline).unwrap();
        let back = load(&store).unwrap();
        assert_eq!(back.get(&id(1)).unwrap().render_mode, RenderMode::Wireframe);
        assert!(back.get(&id(2)).is_some());
        assert!(back.folders().any(|f| f == "F"));
        assert!(merged.get(&id(2)).is_some());
        // A view the window deleted stays deleted, one it edited takes its own version.
        let shrink = ViewBook::from_all(
            vec![elev.clone()],
            ["Plans".to_owned(), "F".to_owned()],
            Vec::new(),
        )
        .unwrap();
        let merged = persist(&store, &shrink, &merged).unwrap();
        assert!(merged.get(&id(2)).is_none());
        let mut mine = elev.clone();
        mine.span_mm = 9000.0;
        let edited =
            ViewBook::from_all(vec![mine], ["Plans".to_owned(), "F".to_owned()], Vec::new())
                .unwrap();
        let merged = persist(&store, &edited, &merged).unwrap();
        assert_eq!(merged.get(&id(1)).unwrap().span_mm, 9000.0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_window_edit_keeps_what_another_process_changed_in_the_same_view() {
        // The 584 recheck on master: the window ticked Favourite on Elev without reloading, and
        // its whole stale copy replaced the render mode and colour the CLI had set meanwhile.
        let dir =
            std::env::temp_dir().join(format!("beamcraft-views-ui-fields-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join("m.store");
        std::fs::create_dir_all(&store).unwrap();
        let elev = view(1, "Elev", "");
        let baseline = ViewBook::from_all(vec![elev.clone()], Vec::new(), Vec::new()).unwrap();
        persist(&store, &baseline, &baseline).unwrap();
        // Another process turns Elev wireframe, red and 12000 mm high, and adds AgentView.
        let mut theirs = elev.clone();
        theirs.render_mode = RenderMode::Wireframe;
        theirs.color = Some([1.0, 0.0, 0.0]);
        theirs.span_mm = 12_000.0;
        let cli = ViewBook::from_all(
            vec![theirs, view(2, "AgentView", "")],
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        std::fs::write(
            session_path(&store),
            session::views_layer::layer_bytes(&cli).unwrap(),
        )
        .unwrap();
        // The window, which never reloaded, ticks Favourite and sets a height of its own.
        let mut ours = elev.clone();
        ours.favorite = true;
        ours.span_mm = 9000.0;
        let book = ViewBook::from_all(vec![ours], Vec::new(), Vec::new()).unwrap();
        let (merged, shared) = persist_merging(&store, &book, &baseline).unwrap();
        assert_eq!(
            shared,
            ["Elev".to_owned()],
            "the window names the merged view"
        );
        for (what, book) in [("merged", merged), ("file", load(&store).unwrap())] {
            let elev = book.get(&id(1)).unwrap();
            assert_eq!(
                elev.render_mode,
                RenderMode::Wireframe,
                "{what}: the CLI's mode"
            );
            assert_eq!(
                elev.color,
                Some([1.0, 0.0, 0.0]),
                "{what}: the CLI's colour"
            );
            assert!(elev.favorite, "{what}: the window's favourite");
            assert_eq!(
                elev.span_mm, 9000.0,
                "{what}: both changed it, the window wins"
            );
            assert!(book.get(&id(2)).is_some(), "{what}: the CLI's new view");
        }
        // Writing again over what the window now holds merges nothing more.
        let reread = load(&store).unwrap();
        let (_, shared) = persist_merging(&store, &reread, &reread).unwrap();
        assert!(
            shared.is_empty(),
            "nothing changed on both sides: {shared:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_unchanged_form_applies_back_bit_for_bit() {
        // The form shows the height to the millimetre, the colour and opacity in single
        // precision and each rotation as degree text: an Apply that touched none of them keeps
        // the view's own values, so it changes nothing the user did not touch.
        let mut original = view(1, "Elev", "");
        original.span_mm = 8095.3;
        original.color = Some([0.1, 0.2, 0.3]);
        original.opacity = 0.3;
        original.rotation_rad = [0.123_456_789, 0.0];
        let mut applied = original.clone();
        Form::from_view(&original)
            .apply_edits(&mut applied)
            .unwrap();
        assert_eq!(applied, original);
        // A touched field is written: a new height replaces the old one, the rest stays.
        let mut form = Form::from_view(&original);
        form.span = "9000".into();
        let mut applied = original.clone();
        form.apply_edits(&mut applied).unwrap();
        assert_eq!(applied.span_mm, 9000.0);
        assert_eq!(applied.color, original.color);
        assert_eq!(applied.rotation_rad, original.rotation_rad);
    }

    #[test]
    fn a_temporary_view_survives_every_merge() {
        let dir =
            std::env::temp_dir().join(format!("beamcraft-views-ui-temp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join("m.store");
        std::fs::create_dir_all(&store).unwrap();
        let default = view(1, "3D View", "");
        let mut temp = view(2, "Scratch", "");
        temp.temporary = true;
        // The window's book and baseline hold the temporary (as persist_views left them); the
        // file holds the saved views only, so every merge must keep the temporary.
        let mut book = ViewBook::from_all(vec![default.clone()], Vec::new(), Vec::new()).unwrap();
        book.insert(temp.clone()).unwrap();
        let baseline = book.clone();
        std::fs::write(
            session_path(&store),
            session::views_layer::layer_bytes(
                &ViewBook::from_all(vec![default.clone()], Vec::new(), Vec::new()).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        let merged = persist(&store, &book, &baseline).unwrap();
        assert!(merged.get(&id(2)).is_some(), "the first merge keeps it");
        let merged = persist(&store, &book, &merged).unwrap();
        assert!(merged.get(&id(2)).is_some(), "the second merge keeps it");
        // A saved view the file named like the temporary refuses the merge loudly instead of
        // dropping the temporary silently.
        let foreign = view(3, "Scratch", "");
        let disk = ViewBook::from_all(vec![default, foreign], Vec::new(), Vec::new()).unwrap();
        std::fs::write(
            session_path(&store),
            session::views_layer::layer_bytes(&disk).unwrap(),
        )
        .unwrap();
        assert!(persist(&store, &book, &merged).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_rotation_field_round_trips_whole_degrees() {
        let mut v = view(3, "Rot", "");
        v.rotation_rad = [30.0_f64.to_radians(), 0.0];
        let form = Form::from_view(&v);
        assert_eq!(form.rotation, ["30".to_owned(), "0".to_owned()]);
        let mut applied = v.clone();
        form.apply(&mut applied).unwrap();
        assert_eq!(applied.rotation_rad, v.rotation_rad);
        assert_eq!(Form::from_view(&applied).rotation, form.rotation);
    }

    #[test]
    fn the_form_round_trips_a_view_and_refuses_bad_numbers() {
        let mut v = view(1, "Section", "A/B");
        v.projection = Projection::Perspective;
        v.depth = Some([300.0, 100.0]);
        v.rotation_rad = [0.5, 0.25];
        v.render_mode = RenderMode::Grey;
        v.hidden_kinds = KindFilter::SHOW_ALL.hiding(ElementKind::Grid);
        v.color = Some([0.5, 0.25, 1.0]);
        v.opacity = 0.5;
        v.favorite = true;
        let form = Form::from_view(&v);
        let mut back = ModelView::new(id(1), "x", WorkPlane::default(), 1.0);
        form.apply(&mut back).unwrap();
        assert_eq!(back.name, "Section");
        assert_eq!(back.folder, "A/B");
        assert_eq!(back.depth, Some([300.0, 100.0]));
        assert!((back.rotation_rad[0] - 0.5).abs() < 1e-12);
        assert_eq!(back.hidden_kinds, v.hidden_kinds);
        assert_eq!(back.color, v.color);
        let mut bad = form.clone();
        bad.span = "tall".into();
        assert!(bad.apply(&mut back).is_err());
        let mut unnamed = form.clone();
        unnamed.name = "  ".into();
        unnamed.apply(&mut back).unwrap();
        assert!(back.temporary, "a view without a name is temporary");
        assert_eq!(back.name, v.name, "the book name stays");
        assert!(Form::from_view(&back).name.is_empty());
        form.apply(&mut back).unwrap();
        assert!(!back.temporary);
        let mut bad = form;
        bad.folder = "a/../b".into();
        assert!(bad.apply(&mut back).is_err());
    }

    #[test]
    fn the_browser_lists_views_by_name_with_their_kind_and_the_default_on_its_own() {
        use beamcraft_engine::work_plane::WorkPlane;
        let mut views = Views::default();
        let mut home = view(1, "Z home", "");
        home.is_default = true;
        home.projection = Projection::Perspective;
        let plan = view(2, "A plan", "");
        let mut elevation = view(3, "Elevation", "");
        elevation.plane = WorkPlane::parallel("xz", 0.0).unwrap();
        let mut section = view(5, "Section A", "");
        section.plane = WorkPlane::parallel("yz", 0.0).unwrap();
        section.depth = Some([500.0, 500.0]);
        // A perspective look at a plan is still a plan: 3D is a way of looking.
        let mut spatial = view(4, "B spatial", "");
        spatial.projection = Projection::Perspective;
        let mut roof = view(6, "Roof", "");
        roof.plane = WorkPlane::new([0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.8, 0.6]).unwrap();
        for v in [plan, elevation, spatial, home, section, roof] {
            views.book.insert(v).unwrap();
        }
        let tree = views.browser_tree();
        // No grouping by kind: one list by name, each view's kind shown by its icon.
        let names: Vec<(String, ViewKind)> = tree
            .views
            .iter()
            .map(|id| {
                let view = views.book.get(id).unwrap();
                (view.name.clone(), view_kind(view))
            })
            .collect();
        assert_eq!(
            names,
            [
                ("A plan".to_owned(), ViewKind::Plan),
                ("B spatial".to_owned(), ViewKind::Plan),
                ("Elevation".to_owned(), ViewKind::Vertical),
                ("Roof".to_owned(), ViewKind::Inclined),
                ("Section A".to_owned(), ViewKind::Vertical),
            ]
        );
        assert_eq!(views.browser_home(), Some(id(1)));
    }

    #[test]
    fn the_browser_tree_nests_folders_filters_and_selects() {
        let mut views = Views::default();
        views
            .book
            .insert(view(1, "b plan", "Plans/Level 1"))
            .unwrap();
        views.book.insert(view(2, "A plan", "")).unwrap();
        let mut fav = view(3, "Old plan", "Archive");
        fav.favorite = true;
        views.book.insert(fav).unwrap();
        views
            .book
            .create_folder(&new_folder_path("Plans", "Level 2"))
            .unwrap();
        views
            .book
            .create_folder(&new_folder_path("", "Grid A"))
            .unwrap();
        let tree = views.browser_tree();
        let plans = &tree.folders[2];
        let folders: Vec<(&str, &str, usize)> = plans
            .folders
            .iter()
            .map(|f| (f.name.as_str(), f.stored.as_str(), f.views.len()))
            .collect();
        assert_eq!(
            folders,
            [
                ("Level 1", "Plans/Level 1", 1),
                ("Level 2", "Plans/Level 2", 0),
            ]
        );
        let top: Vec<&str> = tree.folders.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(top, ["Archive", "Grid A", "Plans"], "folders by name");
        assert_eq!(tree.views, [id(2)]);
        assert_eq!(tree.all_views().len(), 3);
        views.favorites_only = true;
        assert_eq!(views.browser_tree().all_views(), [id(3)]);
        views.favorites_only = false;
        views.search = "PLAN".into();
        let found = views.browser_tree();
        assert_eq!(found.all_views().len(), 3);
        assert!(
            found.folders.iter().all(|f| !f.all_views().is_empty()),
            "empty folders hide"
        );
        // Plain, Ctrl and Shift clicks.
        let order = [id(2), id(1), id(3)];
        assert!(views.click(id(2), false, false, &order));
        assert!(!views.click(id(3), false, true, &order));
        assert_eq!(views.selected.len(), 3);
        views.click(id(1), true, false, &order);
        assert!(!views.selected.contains(&id(1)));
        // Ctrl+Tab cycles the open views.
        views.open = vec![id(1), id(3)];
        assert_eq!(views.active(), None, "no pane is open");
        let camera = Camera::new([0.0; 3], 1000.0, 1.6).unwrap();
        views
            .panes
            .push(crate::panes::ViewPane::new(None, camera, 1));
        views.panes.last_mut().unwrap().view = Some(id(1));
        assert_eq!(views.next_open(), Some(id(3)));
        views.panes.last_mut().unwrap().view = Some(id(3));
        assert_eq!(views.next_open(), Some(id(1)));
        views.forget(&id(1));
        assert_eq!(views.open, [id(3)]);
    }

    #[test]
    fn a_window_cut_adds_planes_and_intersects_areas() {
        use beamcraft_engine::view_clipping::{ClipPlane, Clipping};
        let cut = Clipping {
            planes: vec![ClipPlane::new([0.0; 3], [0.0, 0.0, 1.0]).unwrap()],
            area: Some(WorkArea::new([0.0; 3], [100.0; 3]).unwrap()),
        };
        let mut v = view(1, "Slab", "");
        v.depth = Some([10.0, 10.0]);
        v.work_area = Some(WorkArea::new([50.0; 3], [200.0; 3]).unwrap());
        let both = display_clip(Some(&cut), Some(&v)).unwrap();
        assert_eq!(both.planes.len(), 3);
        let area = both.area.unwrap();
        assert_eq!((area.min, area.max), ([50.0; 3], [100.0; 3]));
        assert!(display_clip(None, Some(&view(2, "All", ""))).is_none());
        assert!(display_clip(None, None).is_none());
        assert_eq!(tint_of(None), crate::camera::NO_TINT);
        let mut tinted = view(3, "T", "");
        tinted.color = Some([1.0, 0.0, 0.0]);
        tinted.opacity = 0.25;
        assert_eq!(tint_of(Some(&tinted)), [1.0, 0.0, 0.0, 0.25]);
    }

    #[test]
    fn picks_count_and_undo() {
        let mut p = Picking {
            construction: Construction::SurfaceEdge,
            ..Picking::default()
        };
        assert_eq!(p.construction.picks(), 3);
        p.face = Some(WorkPlane::default());
        p.points.push([1.0; 3]);
        assert_eq!(p.done(), 2);
        assert!(p.undo() && p.undo());
        assert!(!p.undo());
        assert_eq!(Construction::TwoPoints.label(), "Section by two points");
        assert!(Direction::Front.plane([0.0; 3]).is_ok());
    }

    #[test]
    fn the_panel_starts_on_the_free_view_with_nothing_picked() {
        assert!(!Form::default().picked);
        assert_eq!(Subject::default(), Subject::FreeView(None));
        assert_eq!(Subject::FreeView(Some(1)).title(), "3D view");
        assert_eq!(Subject::NewView.title(), "New view");
        assert_eq!(Subject::SavedView(id(1)).title(), "View properties");
    }
}
