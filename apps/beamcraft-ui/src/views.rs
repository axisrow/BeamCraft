// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V12 model views in the window: the session sidecar, the view-properties form, the browser
//! state and the panes of the work area. A view is display state: the book never touches the
//! loaded snapshot, and the model is never written.

use crate::camera::Camera;
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
    if beamcraft_host::views_layer::looks_like_layer(bytes) {
        return beamcraft_host::views_layer::book_from_layer(bytes)
            .map_err(|e| format!("the saved views were refused: {}", e.message));
    }
    let json = beamcraft_json::decode(bytes, &Limits::DEFAULT)
        .map_err(|_| "the saved views file is not valid JSON".to_owned())?;
    beamcraft_host::views::book_from_json(&json)
        .map_err(|e| format!("the saved views were refused: {e}"))
}

/// Persists the book through a sibling temporary file, so a failed write keeps the old content.
/// Temporary views are not written. A store that is not a directory (no model is open there)
/// has nothing to keep the views beside, so nothing is written.
pub fn persist(store: &Path, book: &ViewBook) -> Result<(), String> {
    if !store.is_dir() {
        return Ok(());
    }
    let path = session_path(store);
    let bytes = beamcraft_host::views_layer::layer_bytes(book)
        .map_err(|e| format!("the saved views were refused: {}", e.message))?;
    if bytes.len() > 1024 * 1024 {
        return Err("the saved views exceed the file limit".to_owned());
    }
    let temp = path.with_extension("usda.tmp");
    std::fs::write(&temp, &bytes)
        .and_then(|()| std::fs::rename(&temp, &path))
        .map_err(|e| format!("the saved views could not be written: {}", e.kind()))
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
        self.span = format!("{}", view.span_mm);
        self.depth_on = view.depth.is_some();
        if let Some([above, below]) = view.depth {
            self.depth = [format!("{above}"), format!("{below}")];
        }
        self.rotation = view.rotation_rad.map(|r| format!("{}", r.to_degrees()));
        self.mode = view.render_mode;
        self.grids = view.grids_visible;
        self.hidden = view.hidden_kinds;
        self.area = view.work_area;
        self.color_on = view.color.is_some();
        if let Some(color) = view.color {
            self.color = color.map(|c| c as f32);
        }
        self.opacity = view.opacity as f32;
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
        view.validate().map_err(|e| e.to_string())
    }

    /// The offset of a coordinate-plane view.
    pub fn plane_offset(&self) -> Result<f64, String> {
        number(&self.offset, "the plane offset")
    }
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

/// What a view looks at, as the browser groups it: a 3D view, a plan, or an elevation or
/// section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ViewKind {
    /// A perspective or oblique view, and the default view.
    ThreeD,
    /// A view straight down (or up) the vertical axis.
    Plan,
    /// A view along a horizontal direction.
    Elevation,
}

impl ViewKind {
    /// The kinds in browser order.
    pub const ALL: [Self; 3] = [Self::ThreeD, Self::Plan, Self::Elevation];

    /// The group title.
    pub fn label(self) -> &'static str {
        match self {
            Self::ThreeD => "3D views",
            Self::Plan => "Plans",
            Self::Elevation => "Sections and elevations",
        }
    }

    /// The kind with this group title.
    pub fn of_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.label() == label)
    }

    /// The icon of the kind.
    pub fn icon(self) -> crate::theme::Icon {
        match self {
            Self::ThreeD => crate::theme::Icon::Block,
            Self::Plan => crate::theme::Icon::Grid,
            Self::Elevation => crate::theme::Icon::ViewFrame,
        }
    }
}

/// The kind of `view`: from its projection and the direction its camera looks along.
pub fn view_kind(view: &ModelView) -> ViewKind {
    if view.is_default || view.projection == Projection::Perspective {
        return ViewKind::ThreeD;
    }
    let normal = view.camera_plane().unwrap_or(view.plane).normal();
    let up = normal[2].abs();
    if up > 0.999 {
        ViewKind::Plan
    } else if up < 0.001 {
        ViewKind::Elevation
    } else {
        ViewKind::ThreeD
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
    /// The pane a primary press just focused, until that button is released: its click only
    /// focuses unless it may select.
    pub focus_press: Option<u64>,
    /// Browser tab.
    pub tab: BrowserTab,
    /// Browser search.
    pub search: String,
    /// Group the browser by folder instead of by view kind.
    pub by_folder: bool,
    /// Show only favourites.
    pub favorites_only: bool,
    /// Show only quick access: favourites, temporary and open views.
    pub quick_access: bool,
    /// Selected views in the browser.
    pub selected: BTreeSet<Uuid7>,
    /// The anchor of a Shift range.
    pub anchor: Option<Uuid7>,
    /// A cut (true) or copied view waiting to be pasted.
    pub clipboard: Option<(Uuid7, bool)>,
    /// Destination of a paste or a new folder.
    pub destination: String,
    /// Whether the new-folder field is shown.
    pub new_folder: bool,
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

/// Most view panes open at once in the work area.
pub const MAX_WINDOWS: usize = 8;
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

    /// The views the browser lists under `search` and the favourites and quick-access switches.
    /// Grouped by what the views look at (3D views, plans, sections and elevations, as a
    /// Revit-style browser does), or, with `by_folder`, by folder (the root first; empty folders
    /// are listed unless a filter is on). Each group is sorted by name, the default view first.
    pub fn browser_groups(&self) -> Vec<(String, Vec<&ModelView>)> {
        let needle = self.search.trim().to_lowercase();
        let mut groups: BTreeMap<String, Vec<&ModelView>> = BTreeMap::new();
        for view in self.book.views() {
            let listed = (!self.favorites_only || view.favorite)
                && (!self.quick_access
                    || view.favorite
                    || view.temporary
                    || self.open.contains(&view.id))
                && format!("{} {}", view.name, view.folder)
                    .to_lowercase()
                    .contains(&needle);
            if listed {
                let key = if self.by_folder {
                    view.folder.clone()
                } else {
                    // The kind's position, so the groups come out in kind order.
                    (view_kind(view) as u8).to_string()
                };
                groups.entry(key).or_default().push(view);
            }
        }
        if self.by_folder && !self.favorites_only && !self.quick_access {
            for folder in self.book.folders() {
                if folder.to_lowercase().contains(&needle) {
                    groups.entry(folder.to_owned()).or_default();
                }
            }
        }
        let mut out: Vec<(String, Vec<&ModelView>)> = groups.into_iter().collect();
        for (key, views) in &mut out {
            views.sort_by_key(|v| (!v.is_default, v.name.to_lowercase()));
            if !self.by_folder {
                *key = key
                    .parse::<usize>()
                    .ok()
                    .and_then(|n| ViewKind::ALL.get(n))
                    .map_or_else(String::new, |k| k.label().to_owned());
            }
        }
        out
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

    /// Forgets a view in the session: its camera, the open and selected marks and the
    /// clipboard. Its panes are closed by the window.
    pub fn forget(&mut self, id: &Uuid7) {
        self.cameras.remove(id);
        self.open.retain(|v| v != id);
        self.selected.remove(id);
        if self.clipboard.is_some_and(|(c, _)| c == *id) {
            self.clipboard = None;
        }
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
        persist(&store, &book).unwrap();
        // The sidecar is the views layer: a camera-prim USD document, not JSON.
        let written = std::fs::read(session_path(&store)).unwrap();
        assert!(beamcraft_host::views_layer::looks_like_layer(&written));
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
        persist(&store, &back).unwrap();
        std::fs::write(legacy_path(&store), b"{broken").unwrap();
        let layer = load(&store).unwrap();
        assert_eq!(layer.len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
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
    fn the_browser_groups_views_by_kind_with_the_default_first() {
        use beamcraft_engine::work_plane::WorkPlane;
        let mut views = Views::default();
        let mut home = view(1, "Z home", "");
        home.is_default = true;
        home.projection = Projection::Perspective;
        let plan = view(2, "A plan", "");
        let mut elevation = view(3, "Elevation", "");
        elevation.plane = WorkPlane::parallel("xz", 0.0).unwrap();
        let mut spatial = view(4, "B spatial", "");
        spatial.projection = Projection::Perspective;
        for v in [plan, elevation, spatial, home] {
            views.book.insert(v).unwrap();
        }
        let groups = views.browser_groups();
        let names: Vec<(&str, Vec<&str>)> = groups
            .iter()
            .map(|(f, v)| (f.as_str(), v.iter().map(|v| v.name.as_str()).collect()))
            .collect();
        assert_eq!(
            names,
            [
                ("3D views", vec!["Z home", "B spatial"]),
                ("Plans", vec!["A plan"]),
                ("Sections and elevations", vec!["Elevation"]),
            ]
        );
    }

    #[test]
    fn the_browser_groups_filters_and_selects() {
        let mut views = Views {
            by_folder: true,
            ..Views::default()
        };
        views.book.insert(view(1, "b plan", "")).unwrap();
        views.book.insert(view(2, "A plan", "")).unwrap();
        let mut fav = view(3, "Section", "Sections");
        fav.favorite = true;
        views.book.insert(fav).unwrap();
        views.book.create_folder("Empty").unwrap();
        let groups = views.browser_groups();
        let names: Vec<(&str, Vec<&str>)> = groups
            .iter()
            .map(|(f, v)| (f.as_str(), v.iter().map(|v| v.name.as_str()).collect()))
            .collect();
        assert_eq!(
            names,
            [
                ("", vec!["A plan", "b plan"]),
                ("Empty", vec![]),
                ("Sections", vec!["Section"])
            ]
        );
        views.favorites_only = true;
        assert_eq!(views.browser_groups().len(), 1);
        views.favorites_only = false;
        views.search = "PLAN".into();
        assert_eq!(views.browser_groups()[0].1.len(), 2);
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
