// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V12 model views in the window: the session sidecar, the view-properties form, the browser
//! state and the extra view windows. A view is display state: the book never touches the loaded
//! snapshot, and the model is never written.

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

/// The session sidecar of one store: a sibling `<store>.views.json` file, never inside the
/// store itself.
pub fn session_path(store: &Path) -> PathBuf {
    let mut name = store.as_os_str().to_os_string();
    name.push(".views.json");
    PathBuf::from(name)
}

/// Loads the book for a store. A missing file is an empty book; anything unreadable is an
/// error note, never a reason to refuse the model.
pub fn load(store: &Path) -> Result<ViewBook, String> {
    let bytes = match std::fs::read(session_path(store)) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ViewBook::new()),
        Err(e) => return Err(format!("the saved views could not be read: {}", e.kind())),
    };
    if bytes.is_empty() {
        return Ok(ViewBook::new());
    }
    if bytes.len() > 1024 * 1024 {
        return Err("the saved views file exceeds 1 MiB".to_owned());
    }
    let json = beamcraft_json::decode(&bytes, &Limits::DEFAULT)
        .map_err(|_| "the saved views file is not valid JSON".to_owned())?;
    beamcraft_host::views::book_from_json(&json)
        .map_err(|e| format!("the saved views were refused: {e}"))
}

/// Persists the book through a sibling temporary file, so a failed write keeps the old content.
/// Temporary views are not written.
pub fn persist(store: &Path, book: &ViewBook) -> Result<(), String> {
    let path = session_path(store);
    let bytes = beamcraft_host::views::book_to_json(book)
        .map_err(|e| format!("the saved views were refused: {e}"))?
        .encode(&Limits::DEFAULT)
        .map_err(|_| "the saved views exceed the JSON limits".to_owned())?;
    let temp = path.with_extension("json.tmp");
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
    /// Name, or the name prefix of a batch.
    pub name: String,
    /// Browser folder.
    pub folder: String,
    /// Quick access mark.
    pub favorite: bool,
    /// Session only.
    pub temporary: bool,
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
}

impl Default for Form {
    fn default() -> Self {
        Self {
            id: None,
            plane: WorkPlane::default(),
            name: String::new(),
            folder: String::new(),
            favorite: false,
            temporary: false,
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
            name: view.name.clone(),
            folder: view.folder.clone(),
            favorite: view.favorite,
            temporary: view.temporary,
            ..Self::default()
        };
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
    /// every display property. The plane is the caller's.
    pub fn apply(&self, view: &mut ModelView) -> Result<(), String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("give the view a name".into());
        }
        name.clone_into(&mut view.name);
        self.folder.trim().clone_into(&mut view.folder);
        validate_folder(&view.folder).map_err(|e| e.to_string())?;
        view.favorite = self.favorite;
        view.temporary = self.temporary;
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

/// The views of one opened model in the window.
#[derive(Default)]
pub struct Views {
    /// The book: saved and temporary views and the folders.
    pub book: ViewBook,
    /// Why the last action failed, or what it did.
    pub note: Option<Result<String, String>>,
    /// The view the main viewport shows; `None` is the free 3D view.
    pub active: Option<Uuid7>,
    /// The session camera of each opened view, so returning to a view keeps its navigation.
    pub cameras: BTreeMap<Uuid7, Camera>,
    /// The free 3D camera and display, kept while a view is active.
    pub spatial: Option<(Camera, RenderMode, bool)>,
    /// The camera Ctrl+P returns to.
    pub toggled: Option<Camera>,
    /// Views opened in this session, in opening order (Ctrl+Tab cycles them).
    pub open: Vec<Uuid7>,
    /// The properties form.
    pub form: Form,
    /// Whether the properties window is shown.
    pub editing: bool,
    /// A pick in progress.
    pub picking: Option<Picking>,
    /// The extra view windows.
    pub windows: Vec<ViewWindow>,
    /// Lay the windows out in a grid on the next frame.
    pub tile: bool,
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
    /// The hidden kinds the main scene was built with.
    pub shown: KindFilter,
    /// The last key handed to a view window.
    pub window_serial: u64,
    /// Whether the browser shows previews.
    pub previews_on: bool,
    /// The rendered previews: the key they were drawn for (snapshot and definition) and the
    /// offscreen target, at most [`MAX_PREVIEWS`].
    pub previews: BTreeMap<Uuid7, (u64, ViewWindow)>,
}

/// Most windows besides the main viewport.
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
    /// The active view.
    pub fn active_view(&self) -> Option<&ModelView> {
        self.active.and_then(|id| self.book.get(&id))
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

    /// The views the browser lists under `search`, the favourites and quick-access switches,
    /// grouped by folder (the root first), each group sorted by name. Empty folders are listed
    /// unless a filter is on.
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
                groups.entry(view.folder.clone()).or_default().push(view);
            }
        }
        if !self.favorites_only && !self.quick_access {
            for folder in self.book.folders() {
                if folder.to_lowercase().contains(&needle) {
                    groups.entry(folder.to_owned()).or_default();
                }
            }
        }
        let mut out: Vec<(String, Vec<&ModelView>)> = groups.into_iter().collect();
        for (_, views) in &mut out {
            views.sort_by_key(|v| v.name.to_lowercase());
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
        let at = self.active.and_then(|a| open.iter().position(|v| *v == a));
        match at {
            Some(i) => open.get((i + 1) % open.len().max(1)).copied(),
            None => open.first().copied(),
        }
    }

    /// Forgets a view in the session: its camera, its window, the active and open marks.
    pub fn forget(&mut self, id: &Uuid7) {
        self.cameras.remove(id);
        self.open.retain(|v| v != id);
        self.selected.remove(id);
        self.windows.retain(|w| w.view != Some(*id));
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

/// Where an extra window sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dock {
    /// Where the user drags it.
    #[default]
    Float,
    /// Along the left edge.
    Left,
    /// Along the right edge.
    Right,
    /// Along the top edge.
    Top,
    /// Along the bottom edge.
    Bottom,
}

/// One extra view window: a model view (or the free 3D view) through its own camera, on its own
/// targets. A window draws and navigates; selection and tools stay in the main viewport.
pub struct ViewWindow {
    /// The view it shows; `None` is a free 3D window.
    pub view: Option<Uuid7>,
    /// Stable identity of the egui window.
    pub(crate) key: u64,
    pub(crate) camera: Camera,
    pub(crate) open: bool,
    pub(crate) dock: Dock,
    /// Render mode, its own.
    pub(crate) mode: RenderMode,
    /// Grid guides shown.
    pub(crate) grids: bool,
    pub(crate) title: String,
    renderer: Option<crate::render::Renderer>,
    texture: Option<egui::TextureId>,
    /// The scene of this window's hidden kinds when they differ from the main scene's, keyed by
    /// the snapshot token it was built for.
    pub(crate) filtered: Option<(String, KindFilter, crate::render::Uploaded)>,
    /// Placement for the next frame (tiling).
    pub(crate) place: Option<egui::Rect>,
}

impl ViewWindow {
    pub(crate) fn new(view: Option<&ModelView>, camera: Camera, index: usize) -> Self {
        Self {
            view: view.map(|v| v.id),
            key: 0,
            camera,
            open: true,
            dock: Dock::Float,
            mode: view.map_or(RenderMode::Shaded, |v| v.render_mode),
            grids: view.is_none_or(|v| v.grids_visible),
            title: view.map_or_else(|| format!("3D view {}", index + 2), |v| v.name.clone()),
            renderer: None,
            texture: None,
            filtered: None,
            place: None,
        }
    }

    /// Resizes or creates the offscreen targets, keeping the egui texture in step. Returns the
    /// aspect of the drawn image.
    pub(crate) fn resize(
        &mut self,
        state: &eframe::egui_wgpu::RenderState,
        width: u32,
        height: u32,
        samples: Option<u32>,
        stats: &crate::stats::Stats,
    ) -> Result<f32, crate::Error> {
        if width == 0 || height == 0 {
            // egui reports a zero-size content rect before the window lays out; wait for real
            // pixels rather than building empty targets.
            return Err(crate::Error::Input("window not laid out yet"));
        }
        if let Some(renderer) = &mut self.renderer {
            if (renderer.width != width || renderer.height != height)
                && renderer.resize(&state.device, width, height, stats)?
                && let Some(texture) = self.texture
            {
                // The targets changed: egui's copy of the texture follows them.
                state
                    .renderer
                    .write()
                    .update_egui_texture_from_wgpu_texture(
                        &state.device,
                        &renderer.view,
                        wgpu::FilterMode::Linear,
                        texture,
                    );
            }
        } else {
            let renderer = crate::render::Renderer::with_samples(
                &state.adapter,
                &state.device,
                width,
                height,
                samples,
                stats,
            )?;
            let texture = state.renderer.write().register_native_texture(
                &state.device,
                &renderer.view,
                wgpu::FilterMode::Linear,
            );
            self.texture = Some(texture);
            self.renderer = Some(renderer);
        }
        let renderer = self.renderer.as_ref().ok_or(crate::Error::NoAdapter)?;
        Ok((renderer.width as f32 / renderer.height as f32).clamp(0.05, 20.0))
    }

    /// The egui texture of the drawn image, once targets exist.
    pub(crate) fn texture(&self) -> Option<egui::TextureId> {
        self.texture
    }

    pub(crate) fn renderer(&self) -> Option<&crate::render::Renderer> {
        self.renderer.as_ref()
    }

    /// Frees the egui texture of a closed window.
    pub(crate) fn release(&mut self, state: &eframe::egui_wgpu::RenderState) {
        if let Some(texture) = self.texture.take() {
            state.renderer.write().free_texture(&texture);
        }
    }
}

/// The rectangle of window `index` of `count` tiled in two columns over `area`.
pub fn tile_rect(area: egui::Rect, index: usize, count: usize) -> egui::Rect {
    let columns = 2usize.min(count.max(1));
    let rows = count.max(1).div_ceil(columns);
    let width = area.width() / columns as f32;
    let height = area.height() / rows as f32;
    let column = (index % columns) as f32;
    let row = (index / columns) as f32;
    egui::Rect::from_min_size(
        area.min + egui::vec2(column * width, row * height),
        egui::vec2(width, height),
    )
    .shrink(4.0)
}

/// The rectangle a docked window takes inside `area`.
pub fn dock_rect(area: egui::Rect, dock: Dock) -> Option<egui::Rect> {
    let (w, h) = (area.width(), area.height());
    let r = match dock {
        Dock::Float => return None,
        Dock::Left => egui::Rect::from_min_size(area.min, egui::vec2(w * 0.4, h)),
        Dock::Right => {
            egui::Rect::from_min_size(area.min + egui::vec2(w * 0.6, 0.0), egui::vec2(w * 0.4, h))
        }
        Dock::Top => egui::Rect::from_min_size(area.min, egui::vec2(w, h * 0.4)),
        Dock::Bottom => {
            egui::Rect::from_min_size(area.min + egui::vec2(0.0, h * 0.6), egui::vec2(w, h * 0.4))
        }
    };
    Some(r.shrink(4.0))
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
        assert_eq!(session_path(&store), dir.join("m.store.views.json"));
        let mut book = ViewBook::new();
        book.insert(view(1, "Plan", "Plans")).unwrap();
        let mut temp = view(2, "Scratch", "");
        temp.temporary = true;
        book.insert(temp).unwrap();
        persist(&store, &book).unwrap();
        let back = load(&store).unwrap();
        assert_eq!(back.len(), 1, "temporary views are not written");
        assert_eq!(back.get(&id(1)), book.get(&id(1)));
        std::fs::write(session_path(&store), b"{not json").unwrap();
        assert!(load(&store).is_err());
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
        let mut bad = form.clone();
        bad.name = "  ".into();
        assert!(bad.apply(&mut back).is_err());
        let mut bad = form;
        bad.folder = "a/../b".into();
        assert!(bad.apply(&mut back).is_err());
    }

    #[test]
    fn the_browser_groups_filters_and_selects() {
        let mut views = Views::default();
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
        views.active = Some(id(1));
        assert_eq!(views.next_open(), Some(id(3)));
        views.active = Some(id(3));
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
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        assert!(tile_rect(rect, 3, 4).min.y > 290.0);
        assert!(dock_rect(rect, Dock::Float).is_none());
        assert!(dock_rect(rect, Dock::Right).unwrap().min.x > 470.0);
        assert!(Direction::Front.plane([0.0; 3]).is_ok());
    }
}
