// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V12 model views: a bounded book of named views, each a view plane with its own display
//! properties (projection, visible height, depth above and below the plane, work area, render
//! mode, hidden kinds, colour and opacity), plus the constructors that derive a view plane from
//! the model: coordinate planes, two or three points, grid axes, levels, part and assembly
//! frames and picked surfaces.
//!
//! A view is display state only: nothing here reads or writes a store, so saving or applying a
//! view can never change the model or its snapshot token. The JSON session format lives in
//! `beamcraft_host::views`, the one layer both the CLI and the window share.

use crate::query::pick::{Hit, SelectionFilter};
use crate::view_clipping::{ClipPlane, Clipping};
use crate::work_plane::WorkPlane;
use crate::{ElementId, KindFilter, RenderMode, ViewSnapshot, grid_query};
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::grid::Grid;
use beamcraft_model::id::Uuid7;
use std::collections::BTreeSet;

/// The axis-aligned model box a view works inside, in millimetres. One type serves the views and
/// the cut plane: V15's [`crate::view_clipping::WorkArea`] is the admission and clipping source
/// of truth.
pub use crate::view_clipping::WorkArea;

/// Saved views per model.
pub const MAX_VIEWS: usize = 256;
/// Browser folders per model, ancestors included.
pub const MAX_FOLDERS: usize = 512;
/// Most `/`-separated segments in a folder path.
pub const MAX_FOLDER_DEPTH: usize = 8;
/// Most characters in a view name.
pub const MAX_NAME_CHARS: usize = 128;
/// Most bytes in a folder path.
const MAX_FOLDER_BYTES: usize = 512;
/// Visible height bounds, millimetres.
pub const MIN_SPAN_MM: f64 = 1.0;
/// Largest visible height, millimetres.
pub const MAX_SPAN_MM: f64 = 2e7;
/// Largest depth above or below the view plane, millimetres.
pub const MAX_DEPTH_MM: f64 = 2e7;
/// Most parts in an assembly view.
pub const MAX_GROUP_PARTS: usize = 65_536;
/// Visible height of a new view when the caller has nothing better.
pub const DEFAULT_SPAN_MM: f64 = 10_000.0;
/// The window's default 3D orbit: yaw and pitch in radians.
pub const DEFAULT_ORBIT: [f64; 2] = [-0.8, 0.85];

fn invalid(field: &'static str, message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::InvalidArgument, field, message)
}

fn not_found(field: &'static str, message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::NotFound, field, message)
}

fn sub([ax, ay, az]: [f64; 3], [bx, by, bz]: [f64; 3]) -> [f64; 3] {
    [ax - bx, ay - by, az - bz]
}

fn add([ax, ay, az]: [f64; 3], [bx, by, bz]: [f64; 3]) -> [f64; 3] {
    [ax + bx, ay + by, az + bz]
}

fn scale([x, y, z]: [f64; 3], s: f64) -> [f64; 3] {
    [x * s, y * s, z * s]
}

fn dot([ax, ay, az]: [f64; 3], [bx, by, bz]: [f64; 3]) -> f64 {
    ax * bx + ay * by + az * bz
}

fn cross([ax, ay, az]: [f64; 3], [bx, by, bz]: [f64; 3]) -> [f64; 3] {
    [ay * bz - az * by, az * bx - ax * bz, ax * by - ay * bx]
}

fn length(v: [f64; 3]) -> f64 {
    dot(v, v).sqrt()
}

/// Camera projection of a view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Projection {
    /// Parallel rays and a constant scale with depth.
    #[default]
    Orthographic,
    /// A 45-degree vertical perspective field of view.
    Perspective,
}

impl Projection {
    /// The stable wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Orthographic => "orthographic",
            Self::Perspective => "perspective",
        }
    }

    /// Parses an exact token.
    pub fn parse(text: &str) -> Result<Self, CommandError> {
        match text {
            "orthographic" => Ok(Self::Orthographic),
            "perspective" => Ok(Self::Perspective),
            _ => Err(invalid(
                "views.projection",
                "projection must be orthographic or perspective",
            )),
        }
    }
}

/// Admits a browser folder path: at most eight nonempty, unpadded `/`-separated segments, no
/// `.` or `..`, no control characters. The empty path is the root. It is never a filesystem path.
pub fn validate_folder(folder: &str) -> Result<(), CommandError> {
    if folder.len() > MAX_FOLDER_BYTES
        || folder.chars().any(char::is_control)
        || folder.split('/').count() > MAX_FOLDER_DEPTH
        || (!folder.is_empty()
            && folder
                .split('/')
                .any(|s| s.trim() != s || s.is_empty() || matches!(s, "." | "..")))
    {
        return Err(invalid(
            "views.folder",
            "a folder needs at most eight nonempty path segments",
        ));
    }
    Ok(())
}

/// Every ancestor of `folder`, the folder itself last; nothing for the root.
pub fn folder_ancestors(folder: &str) -> Vec<String> {
    if folder.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut path = String::new();
    for segment in folder.split('/') {
        if !path.is_empty() {
            path.push('/');
        }
        path.push_str(segment);
        out.push(path.clone());
    }
    out
}

/// Whether `folder` is `parent` or lies below it.
pub fn folder_within(folder: &str, parent: &str) -> bool {
    folder == parent
        || folder
            .strip_prefix(parent)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// One named view: its identity, name, view plane and display properties, in model millimetres.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelView {
    /// Persistent UUIDv7 identity.
    pub id: Uuid7,
    /// User-visible name, unique in the book (case-insensitively).
    pub name: String,
    /// View plane and input frame. Local +Z points towards the viewer.
    pub plane: WorkPlane,
    /// Camera projection.
    pub projection: Projection,
    /// Visible height at the view plane.
    pub span_mm: f64,
    /// Visible distance `[above, below]` the plane; `above` is towards the viewer. `None` shows
    /// everything in front of and behind the plane.
    pub depth: Option<[f64; 2]>,
    /// Display mode of this view.
    pub render_mode: RenderMode,
    /// Whether grid guides are displayed.
    pub grids_visible: bool,
    /// Browser folder path, `/`-separated; empty is the root.
    pub folder: String,
    /// Listed in quick access.
    pub favorite: bool,
    /// Session-only: never written to the saved book.
    pub temporary: bool,
    /// Element kinds hidden in this view.
    pub hidden_kinds: KindFilter,
    /// Optional world-space box the view shows.
    pub work_area: Option<WorkArea>,
    /// Camera rotations about the view's local Z, then X, in radians. The input plane stays fixed.
    pub rotation_rad: [f64; 2],
    /// Optional display colour override, linear RGB in `0..=1`.
    pub color: Option<[f64; 3]>,
    /// Display opacity in `0..=1`.
    pub opacity: f64,
}

impl ModelView {
    /// A view with the default display properties: orthographic, no depth limit, shaded, grids
    /// shown, no work area, no rotation, model colours, opaque.
    pub fn new(id: Uuid7, name: &str, plane: WorkPlane, span_mm: f64) -> Self {
        Self {
            id,
            name: name.trim().to_owned(),
            plane,
            projection: Projection::Orthographic,
            span_mm,
            depth: None,
            render_mode: RenderMode::Shaded,
            grids_visible: true,
            folder: String::new(),
            favorite: false,
            temporary: false,
            hidden_kinds: KindFilter::SHOW_ALL,
            work_area: None,
            rotation_rad: [0.0; 2],
            color: None,
            opacity: 1.0,
        }
    }

    /// Copies the display properties of `source`; identity, name, plane, folder and the
    /// favourite and temporary marks stay.
    pub fn apply_display_properties(&mut self, source: &Self) {
        self.projection = source.projection;
        self.span_mm = source.span_mm;
        self.depth = source.depth;
        self.render_mode = source.render_mode;
        self.grids_visible = source.grids_visible;
        self.hidden_kinds = source.hidden_kinds;
        self.work_area = source.work_area;
        self.rotation_rad = source.rotation_rad;
        self.color = source.color;
        self.opacity = source.opacity;
    }

    /// Admits the view: name, span, depth, rotation, colour, opacity, folder and work area.
    pub fn validate(&self) -> Result<(), CommandError> {
        if self.name.trim() != self.name
            || self.name.is_empty()
            || self.name.chars().count() > MAX_NAME_CHARS
            || self.name.chars().any(char::is_control)
        {
            return Err(invalid(
                "views.name",
                "a view name needs 1 to 128 characters without outer spaces or controls",
            ));
        }
        if !self.span_mm.is_finite() || !(MIN_SPAN_MM..=MAX_SPAN_MM).contains(&self.span_mm) {
            return Err(invalid(
                "views.span",
                "the visible height must be between 1 and 20000000 mm",
            ));
        }
        if self
            .rotation_rad
            .iter()
            .any(|v| !v.is_finite() || v.abs() > std::f64::consts::TAU * 1000.0)
            || !self.opacity.is_finite()
            || !(0.0..=1.0).contains(&self.opacity)
            || self
                .color
                .is_some_and(|c| c.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)))
        {
            return Err(invalid(
                "views.display",
                "camera rotation, colour or opacity is out of range",
            ));
        }
        validate_folder(&self.folder)?;
        if let Some(area) = self.work_area {
            WorkArea::new(area.min, area.max)?;
        }
        if let Some([above, below]) = self.depth
            && (!above.is_finite()
                || !below.is_finite()
                || !(0.0..=MAX_DEPTH_MM).contains(&above)
                || !(0.0..=MAX_DEPTH_MM).contains(&below)
                || above + below <= 0.0)
        {
            return Err(invalid(
                "views.depth",
                "view depth needs bounded nonnegative distances and a positive thickness",
            ));
        }
        WorkPlane::new(self.plane.origin(), self.plane.x(), self.plane.y())?;
        Ok(())
    }

    /// Whether a world point is shown: inside the work area and the depth slab, and the view is
    /// not fully transparent.
    pub fn contains(&self, point: [f64; 3]) -> bool {
        if self.opacity == 0.0 || point.iter().any(|v| !v.is_finite()) {
            return false;
        }
        if let Some(area) = self.work_area
            && point
                .into_iter()
                .zip(area.min.into_iter().zip(area.max))
                .any(|(v, (lo, hi))| v < lo || v > hi)
        {
            return false;
        }
        let Some([above, below]) = self.depth else {
            return true;
        };
        let d = self.plane_distance(point);
        d >= -below && d <= above
    }

    /// Signed distance of a world point from the view plane, positive towards the viewer.
    pub fn plane_distance(&self, point: [f64; 3]) -> f64 {
        dot(sub(point, self.plane.origin()), self.plane.normal())
    }

    /// Whether an element of `kind` is shown.
    pub fn shows_kind(&self, kind: crate::ElementKind) -> bool {
        self.hidden_kinds.is_visible(kind)
    }

    /// The view's display cut: the depth slab as two half-spaces and the work area as a box,
    /// for the renderer's clip planes. Empty when the view shows everything.
    pub fn clipping(&self) -> Result<Clipping, CommandError> {
        let mut planes = Vec::new();
        if let Some([above, below]) = self.depth {
            let o = self.plane.origin();
            let n = self.plane.normal();
            let at = |d: f64| add(o, scale(n, d));
            planes.push(ClipPlane::new(at(above), n.map(|v| -v))?);
            planes.push(ClipPlane::new(at(-below), n)?);
        }
        let clipping = Clipping {
            planes,
            area: self.work_area,
        };
        clipping.validate()?;
        Ok(clipping)
    }

    /// The camera frame: the view plane turned by the rotations about its local Z, then X. The
    /// view plane itself, the input frame, never moves.
    pub fn camera_plane(&self) -> Result<WorkPlane, CommandError> {
        self.validate()?;
        let [z, x] = self.rotation_rad;
        let (sz, cz) = z.sin_cos();
        let (sx, cx) = x.sin_cos();
        let (px, py, pn) = (self.plane.x(), self.plane.y(), self.plane.normal());
        let right = add(scale(px, cz), scale(py, sz));
        let up = add(scale(add(scale(px, -sz), scale(py, cz)), cx), scale(pn, sx));
        WorkPlane::new(self.plane.origin(), right, up)
    }
}

/// The named views of one model and their browser folders. Views keep their insertion order;
/// names are unique case-insensitively and identities are unique.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewBook {
    views: Vec<ModelView>,
    folders: BTreeSet<String>,
}

impl ViewBook {
    /// An empty book.
    pub fn new() -> Self {
        Self::default()
    }

    /// Admits a whole book: every view, unique names and identities, the view and folder limits.
    pub fn from_parts(
        views: Vec<ModelView>,
        folders: impl IntoIterator<Item = String>,
    ) -> Result<Self, CommandError> {
        let mut book = Self::new();
        for folder in folders {
            book.create_folder_tree(&folder)?;
        }
        for view in views {
            book.insert(view)?;
        }
        Ok(book)
    }

    /// The views in book order.
    pub fn views(&self) -> &[ModelView] {
        &self.views
    }

    /// Every folder, ancestors included, in path order.
    pub fn folders(&self) -> impl Iterator<Item = &str> {
        self.folders.iter().map(String::as_str)
    }

    /// How many views the book holds.
    pub fn len(&self) -> usize {
        self.views.len()
    }

    /// Whether the book holds no view.
    pub fn is_empty(&self) -> bool {
        self.views.is_empty()
    }

    /// The view with this identity.
    pub fn get(&self, id: &Uuid7) -> Option<&ModelView> {
        self.views.iter().find(|v| v.id == *id)
    }

    /// The view named by `selector`: its identity, or its exact name (case-insensitively).
    pub fn find(&self, selector: &str) -> Option<&ModelView> {
        if let Ok(id) = Uuid7::parse(selector)
            && let Some(view) = self.get(&id)
        {
            return Some(view);
        }
        let wanted = selector.trim().to_lowercase();
        self.views.iter().find(|v| v.name.to_lowercase() == wanted)
    }

    fn name_taken(&self, name: &str, except: Option<&Uuid7>) -> bool {
        let wanted = name.to_lowercase();
        self.views
            .iter()
            .any(|v| Some(&v.id) != except && v.name.to_lowercase() == wanted)
    }

    fn create_folder_tree(&mut self, folder: &str) -> Result<(), CommandError> {
        validate_folder(folder)?;
        let fresh: Vec<String> = folder_ancestors(folder)
            .into_iter()
            .filter(|f| !self.folders.contains(f))
            .collect();
        if self.folders.len() + fresh.len() > MAX_FOLDERS {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "views.folders",
                "the model already has the maximum number of view folders",
            ));
        }
        self.folders.extend(fresh);
        Ok(())
    }

    /// Adds a new view; refuses a taken identity or name and the view limit. Its folder is
    /// created with its ancestors.
    pub fn insert(&mut self, view: ModelView) -> Result<(), CommandError> {
        view.validate()?;
        if self.get(&view.id).is_some() || self.name_taken(&view.name, None) {
            return Err(CommandError::new(
                ErrorCode::DuplicateId,
                "views.name",
                "view names and identities must be unique",
            ));
        }
        if self.views.len() >= MAX_VIEWS {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "views.count",
                "the model already has the maximum number of views",
            ));
        }
        self.create_folder_tree(&view.folder)?;
        self.views.push(view);
        Ok(())
    }

    /// Adds several new views at once: all of them or, on any refusal, none.
    pub fn insert_all(&mut self, views: Vec<ModelView>) -> Result<(), CommandError> {
        let mut next = self.clone();
        for view in views {
            next.insert(view)?;
        }
        *self = next;
        Ok(())
    }

    /// Replaces the view with the same identity; refuses an unknown identity or a name another
    /// view already has.
    pub fn update(&mut self, view: ModelView) -> Result<(), CommandError> {
        view.validate()?;
        if self.name_taken(&view.name, Some(&view.id)) {
            return Err(CommandError::new(
                ErrorCode::DuplicateId,
                "views.name",
                "another view already has this name",
            ));
        }
        let mut next = self.folders.clone();
        let fresh: Vec<String> = folder_ancestors(&view.folder)
            .into_iter()
            .filter(|f| !next.contains(f))
            .collect();
        if next.len() + fresh.len() > MAX_FOLDERS {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "views.folders",
                "the model already has the maximum number of view folders",
            ));
        }
        let slot = self
            .views
            .iter_mut()
            .find(|v| v.id == view.id)
            .ok_or_else(|| not_found("views.id", "VIEW_UNKNOWN"))?;
        next.extend(fresh);
        *slot = view;
        self.folders = next;
        Ok(())
    }

    /// Removes a view by identity, returning it.
    pub fn remove(&mut self, id: &Uuid7) -> Option<ModelView> {
        let at = self.views.iter().position(|v| v.id == *id)?;
        Some(self.views.remove(at))
    }

    /// Creates a folder with its ancestors; refuses the root and an existing folder.
    pub fn create_folder(&mut self, folder: &str) -> Result<(), CommandError> {
        if folder.is_empty() {
            return Err(invalid("views.folder", "the root folder always exists"));
        }
        validate_folder(folder)?;
        if self.folders.contains(folder) {
            return Err(CommandError::new(
                ErrorCode::DuplicateId,
                "views.folder",
                "the folder already exists",
            ));
        }
        self.create_folder_tree(folder)
    }

    /// Deletes an empty folder and its (empty) subfolders; refuses one that still holds views.
    pub fn delete_folder(&mut self, folder: &str) -> Result<(), CommandError> {
        if folder.is_empty() || !self.folders.contains(folder) {
            return Err(not_found("views.folder", "the folder does not exist"));
        }
        if self.views.iter().any(|v| folder_within(&v.folder, folder)) {
            return Err(CommandError::new(
                ErrorCode::DependencyConflict,
                "views.folder",
                "the folder still contains views",
            ));
        }
        self.folders.retain(|f| !folder_within(f, folder));
        Ok(())
    }

    /// `base` when no view has that name, else `base (2)`, `base (3)` ... The names in `extra`
    /// count as taken too (a batch's earlier names).
    pub fn unique_name(&self, base: &str, extra: &[String]) -> String {
        let taken = |name: &str| {
            self.name_taken(name, None)
                || extra
                    .iter()
                    .any(|e| e.to_lowercase() == name.to_lowercase())
        };
        if !taken(base) {
            return base.to_owned();
        }
        (2..)
            .map(|n| format!("{base} ({n})"))
            .find(|name| !taken(name))
            .unwrap_or_else(|| base.to_owned())
    }
}

/// The view plane a free 3D camera orbit looks at: `target` is the origin, the local +Z points
/// towards the eye at `yaw` and `pitch` (radians), local Y is screen up.
pub fn orbit_plane(target: [f64; 3], yaw: f64, pitch: f64) -> Result<WorkPlane, CommandError> {
    if !yaw.is_finite() || !pitch.is_finite() {
        return Err(CommandError::new(
            ErrorCode::NonFiniteNumber,
            "views.orbit",
            "orbit angles must be finite",
        ));
    }
    let (s, c) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    WorkPlane::new(target, [-s, c, 0.0], [-c * sp, -s * sp, cp])
}

/// The model's coordinate plane `xy`, `xz` or `yz` at `offset` millimetres.
pub fn coordinate_plane(name: &str, offset: f64) -> Result<WorkPlane, CommandError> {
    WorkPlane::parallel(name, offset)
}

/// A vertical plane through two points, local Y up along world Z.
pub fn two_points(a: [f64; 3], b: [f64; 3]) -> Result<WorkPlane, CommandError> {
    WorkPlane::vertical(a, b)
}

/// A section through two points perpendicular to the active input plane: local X runs from `a`
/// towards `b` projected into the input plane, local Y is the input plane's normal.
pub fn two_points_in_plane(
    a: [f64; 3],
    b: [f64; 3],
    input: WorkPlane,
) -> Result<WorkPlane, CommandError> {
    WorkPlane::new(a, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0])?;
    WorkPlane::new(b, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0])?;
    let n = input.normal();
    let ab = sub(b, a);
    WorkPlane::new(a, sub(ab, scale(n, dot(ab, n))), n)
}

/// A plane through three points: origin `a`, +X towards `b`, +Y on the side of `c`.
pub fn three_points(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> Result<WorkPlane, CommandError> {
    WorkPlane::three_points(a, b, c)
}

fn grid_line_plane(grid: &Grid, line: &grid_query::GridSegment) -> Result<WorkPlane, CommandError> {
    let [ax, ay] = grid_query::local_to_world(grid, line.start);
    let [bx, by] = grid_query::local_to_world(grid, line.end);
    two_points([ax, ay, 0.0], [bx, by, 0.0])
}

/// A vertical plane along one straight axis of `grid`, named by label or identity; rotated and
/// drawn grids included. An ambiguous label is refused.
pub fn grid_axis(grid: &Grid, axis: &str) -> Result<WorkPlane, CommandError> {
    let lines = grid_query::segments(grid)?;
    let mut matching = lines
        .iter()
        .filter(|line| line.label == axis || line.axis_id.to_string() == axis);
    let line = matching
        .next()
        .ok_or_else(|| not_found("views.axis", "GRID_AXIS_UNKNOWN"))?;
    if matching.next().is_some() {
        return Err(invalid(
            "views.axis",
            "the axis label is ambiguous; use its identity",
        ));
    }
    grid_line_plane(grid, line)
}

/// Which axes of one grid family a batch takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisPick {
    /// No axis.
    None,
    /// The axis nearest the grid's local origin.
    First,
    /// The axis farthest from the grid's local origin.
    Last,
    /// Every axis.
    All,
}

impl AxisPick {
    /// The stable wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::First => "first",
            Self::Last => "last",
            Self::All => "all",
        }
    }

    /// Parses an exact token.
    pub fn parse(text: &str) -> Result<Self, CommandError> {
        match text {
            "none" => Ok(Self::None),
            "first" => Ok(Self::First),
            "last" => Ok(Self::Last),
            "all" => Ok(Self::All),
            _ => Err(invalid(
                "views.axes",
                "an axis choice is none, first, last or all",
            )),
        }
    }
}

/// One grid axis a batch selected, and the plane through it.
#[derive(Debug, Clone, PartialEq)]
pub struct GridViewAxis {
    /// The family: U is 0, V is 1.
    pub family: u8,
    /// The axis label.
    pub label: String,
    /// The vertical view plane.
    pub plane: WorkPlane,
}

/// The axes of each family a batch takes, U first, then V. First and last are by perpendicular
/// distance from the grid's local origin, ties by axis identity.
pub fn grid_views(
    grid: &Grid,
    u: AxisPick,
    v: AxisPick,
) -> Result<Vec<GridViewAxis>, CommandError> {
    let lines = grid_query::segments(grid)?;
    let mut result = Vec::new();
    for (family, pick) in [(0u8, u), (1u8, v)] {
        let mut selected: Vec<&grid_query::GridSegment> =
            lines.iter().filter(|l| l.family == family).collect();
        let distance = |line: &&grid_query::GridSegment| {
            let [x, y] = line.start;
            let [ex, ey] = line.end;
            let len = (ex - x).hypot(ey - y);
            if len > 0.0 {
                (x * (ey - y) - y * (ex - x)).abs() / len
            } else {
                x.hypot(y)
            }
        };
        match pick {
            AxisPick::None => selected.clear(),
            AxisPick::All => {}
            AxisPick::First | AxisPick::Last => {
                selected.sort_by(|a, b| {
                    distance(a)
                        .total_cmp(&distance(b))
                        .then_with(|| a.axis_id.cmp(&b.axis_id))
                });
                if pick == AxisPick::Last {
                    selected.reverse();
                }
                selected.truncate(1);
            }
        }
        for line in selected {
            result.push(GridViewAxis {
                family,
                label: line.label.clone(),
                plane: grid_line_plane(grid, line)?,
            });
        }
    }
    Ok(result)
}

/// One model level a plan view can show.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelChoice {
    /// The level's identity.
    pub id: ElementId,
    /// Its label.
    pub name: String,
    /// Its elevation, millimetres.
    pub elevation_mm: f64,
}

/// The plan plane of a level: the model XY plane at its elevation.
pub fn level_plane(level: &LevelChoice) -> Result<WorkPlane, CommandError> {
    WorkPlane::parallel("xy", level.elevation_mm)
}

/// The level named by `selector` (identity or exact label); an ambiguous label is refused.
pub fn find_level<'a>(
    levels: &'a [LevelChoice],
    selector: &str,
) -> Result<&'a LevelChoice, CommandError> {
    let mut matching = levels
        .iter()
        .filter(|l| l.name == selector || l.id.to_string() == selector);
    let level = matching
        .next()
        .ok_or_else(|| not_found("views.level", "the level does not exist"))?;
    if matching.next().is_some() {
        return Err(invalid(
            "views.level",
            "the level label is ambiguous; use its identity",
        ));
    }
    Ok(level)
}

/// The levels a plan batch takes: all, or the first or last by distance from elevation zero
/// (ties by name), in that order.
pub fn level_batch(levels: &[LevelChoice], pick: AxisPick) -> Vec<&LevelChoice> {
    let mut sorted: Vec<&LevelChoice> = levels.iter().collect();
    sorted.sort_by(|a, b| {
        a.elevation_mm
            .abs()
            .total_cmp(&b.elevation_mm.abs())
            .then_with(|| a.name.cmp(&b.name))
    });
    match pick {
        AxisPick::None => Vec::new(),
        AxisPick::All => sorted,
        AxisPick::First => sorted.into_iter().take(1).collect(),
        AxisPick::Last => sorted.into_iter().last().into_iter().collect(),
    }
}

/// Which side a part or assembly view looks from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartSide {
    /// Along +Y of the part, local X to the right.
    Front,
    /// The opposite of front.
    Back,
    /// Down onto the part.
    Top,
    /// Up from below.
    Bottom,
    /// Along the part's X axis.
    End,
    /// The window's default 3D orbit, in the part's frame.
    Spatial,
}

impl PartSide {
    /// The four views of a part set: front, top, end and 3D.
    pub const FOUR: [Self; 4] = [Self::Front, Self::Top, Self::End, Self::Spatial];

    /// The stable wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Front => "front",
            Self::Back => "back",
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::End => "end",
            Self::Spatial => "3d",
        }
    }

    /// Parses an exact token.
    pub fn parse(text: &str) -> Result<Self, CommandError> {
        [
            Self::Front,
            Self::Back,
            Self::Top,
            Self::Bottom,
            Self::End,
            Self::Spatial,
        ]
        .into_iter()
        .find(|s| s.as_str() == text)
        .ok_or_else(|| {
            invalid(
                "views.side",
                "a side is front, back, top, bottom, end or 3d",
            )
        })
    }

    fn frame(self) -> ([f64; 3], [f64; 3]) {
        match self {
            Self::Front => ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            Self::Back => ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            Self::Top => ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            Self::Bottom => ([1.0, 0.0, 0.0], [0.0, -1.0, 0.0]),
            Self::End => ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
            Self::Spatial => {
                let [yaw, pitch] = DEFAULT_ORBIT;
                let (s, c) = yaw.sin_cos();
                let (sp, cp) = pitch.sin_cos();
                ([-s, c, 0.0], [-c * sp, -s * sp, cp])
            }
        }
    }
}

fn framed_span(bounds: crate::Aabb) -> f64 {
    let [ax, ay, az] = bounds.min;
    let [bx, by, bz] = bounds.max;
    ((bx - ax).hypot(by - ay).hypot(bz - az) * 1.25).clamp(MIN_SPAN_MM, MAX_SPAN_MM)
}

fn centre(bounds: crate::Aabb) -> [f64; 3] {
    scale(add(bounds.min, bounds.max), 0.5)
}

/// The view plane of one part seen from `side` in its own frame, centred on its evaluated
/// bounds, and the visible height that frames it.
pub fn part_plane(
    snapshot: &ViewSnapshot,
    id: &ElementId,
    side: PartSide,
) -> Result<(WorkPlane, f64), CommandError> {
    let entry = snapshot
        .element(id)
        .ok_or_else(|| not_found("views.element", "the view target was not found"))?;
    let bounds = entry.full.world_bounds;
    let (x, y) = side.frame();
    let rotate = |v| crate::query::snapshot::rotate_vector(v, entry.orientation_xyzw);
    let plane = WorkPlane::new(centre(bounds), rotate(x), rotate(y))?;
    Ok((plane, framed_span(bounds)))
}

/// The view plane of a group of parts (an assembly) seen from `side` in the first part's frame,
/// centred on the union of their bounds, and the visible height that frames them.
pub fn group_plane(
    snapshot: &ViewSnapshot,
    parts: &[ElementId],
    side: PartSide,
) -> Result<(WorkPlane, f64), CommandError> {
    if parts.is_empty() || parts.len() > MAX_GROUP_PARTS {
        return Err(invalid(
            "views.group",
            "an assembly view needs 1 to 65536 parts",
        ));
    }
    let owner = parts
        .first()
        .ok_or_else(|| invalid("views.group", "the assembly is empty"))?;
    let (plane, _) = part_plane(snapshot, owner, side)?;
    let mut bounds: Option<crate::Aabb> = None;
    for id in parts {
        let next = snapshot
            .element_bounds(id)
            .map_err(|_| invalid("views.group", "a part's geometry is unavailable"))?;
        bounds = Some(bounds.map_or(next, |old| old.union(next)));
    }
    let bounds = bounds.ok_or_else(|| invalid("views.group", "the assembly is empty"))?;
    Ok((
        WorkPlane::new(centre(bounds), plane.x(), plane.y())?,
        framed_span(bounds),
    ))
}

/// The view plane on the planar face a ray hits first. Its X runs along `edge` when given (both
/// ends on the face's plane), else along the longest boundary edge of the face's element that
/// lies in the plane. With `view`, only geometry visible in that view can be hit.
pub fn surface_plane(
    snapshot: &ViewSnapshot,
    view: Option<&ModelView>,
    origin: [f64; 3],
    direction: [f64; 3],
    edge: Option<[[f64; 3]; 2]>,
) -> Result<WorkPlane, CommandError> {
    use crate::query::feature::{FeatureGeometry, FeatureKind, lookup_feature};
    use crate::query::pick::{PickQuery, RayPick, pick};
    let hit = match view {
        Some(view) => pick_visible(snapshot, view, origin, direction, SelectionFilter::ALL)?,
        None => {
            let mut query = RayPick::new(origin, direction, vec![FeatureKind::Face]);
            query.max_hits = 1;
            pick(snapshot, &PickQuery::Ray(query))
                .map_err(|e| {
                    CommandError::new(e.f04_code(), "views.surface", "surface pick failed")
                })?
                .hits
                .into_iter()
                .next()
        }
    }
    .ok_or_else(|| not_found("views.surface", "no visible planar face was hit"))?;
    let feature = hit
        .feature_id
        .as_ref()
        .ok_or_else(|| invalid("views.surface", "the surface has no planar feature"))?;
    let feature = lookup_feature(snapshot, &feature.to_string())
        .map_err(|_| invalid("views.surface", "the surface feature is unavailable"))?;
    let normal = match feature.world {
        FeatureGeometry::Face { normal, .. }
        | FeatureGeometry::Surface {
            normal: Some(normal),
            ..
        } => normal,
        _ => {
            return Err(CommandError::new(
                ErrorCode::UnsupportedGeometry,
                "views.surface",
                "a surface view needs a planar face",
            ));
        }
    };
    let plane = WorkPlane::face(hit.point_world_mm, normal)?;
    let on_plane = |p: [f64; 3]| dot(sub(p, plane.origin()), plane.normal()).abs() < 1e-5;
    let chosen = match edge {
        Some([a, b]) => {
            if !on_plane(a) || !on_plane(b) {
                return Err(invalid(
                    "views.edge",
                    "the chosen edge must lie on the picked face",
                ));
            }
            Some([a, b])
        }
        None => {
            let length = |[a, b]: [[f64; 3]; 2]| length(sub(b, a));
            crate::query::feature::element_edges(snapshot, &hit.element_id)
                .map_err(|_| invalid("views.surface", "the face boundary is unavailable"))?
                .into_iter()
                .filter(|[a, b]| on_plane(*a) && on_plane(*b))
                .fold(None, |best: Option<[[f64; 3]; 2]>, e| {
                    if best.is_none_or(|old| length(e) > length(old)) {
                        Some(e)
                    } else {
                        best
                    }
                })
        }
    };
    let Some([a, b]) = chosen else {
        return Ok(plane);
    };
    let x = sub(b, a);
    WorkPlane::new(a, x, cross(plane.normal(), x))
}

/// The first face hit along a ray that the view shows: inside its depth slab and work area and
/// of a visible kind. Clipped or hidden geometry in front never masks a visible hit behind it.
pub fn pick_visible(
    snapshot: &ViewSnapshot,
    view: &ModelView,
    origin: [f64; 3],
    direction: [f64; 3],
    selection: SelectionFilter,
) -> Result<Option<Hit>, CommandError> {
    use crate::query::feature::FeatureKind;
    use crate::query::pick::{MAX_HITS, PickQuery, RayPick, pick};
    view.validate()?;
    let mut query = RayPick::new(origin, direction, vec![FeatureKind::Face]);
    query.max_hits = MAX_HITS;
    query.selection = selection;
    let hits = pick(snapshot, &PickQuery::Ray(query))
        .map_err(|e| CommandError::new(e.f04_code(), "views.pick", "the view pick failed"))?;
    if let Some(hit) = hits.hits.into_iter().find(|hit| {
        view.contains(hit.point_world_mm)
            && snapshot
                .element(&hit.element_id)
                .is_some_and(|e| view.shows_kind(e.kind))
    }) {
        return Ok(Some(hit));
    }
    if hits.truncated {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "views.pick",
            "the view pick exceeds the hit budget",
        ));
    }
    Ok(None)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use beamcraft_model::grid::{DrawnGridLine, GridFamily, RegularGridFamily};

    fn id(n: u64) -> Uuid7 {
        Uuid7::parse(&format!("019a7bf0-0000-7000-8000-{n:012}")).unwrap()
    }

    fn eid(n: u64) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-{n:012}")).unwrap()
    }

    fn view(n: u64, name: &str) -> ModelView {
        ModelView::new(id(n), name, WorkPlane::default(), 5000.0)
    }

    fn grid_a() -> Grid {
        let u = GridFamily::Regular(RegularGridFamily {
            expression: "0,4000".into(),
            axis_ids: vec![eid(1), eid(2)],
            labels: vec!["A".to_owned(), "B".to_owned()],
            positions_mm: vec![0.0, 4000.0],
            span_mm: [0.0, 8000.0],
        });
        let v = GridFamily::Regular(RegularGridFamily {
            expression: "0,6000".into(),
            axis_ids: vec![eid(3), eid(4)],
            labels: vec!["1".to_owned(), "2".to_owned()],
            positions_mm: vec![0.0, 6000.0],
            span_mm: [0.0, 12000.0],
        });
        Grid {
            id: eid(99),
            name: "Main".into(),
            u,
            v,
            translation_mm: [500.0, -250.0],
            orientation_zw: [0.0, 1.0],
        }
    }

    #[test]
    fn views_admit_their_properties() {
        let mut v = view(1, "  Plan ");
        assert_eq!(v.name, "Plan");
        v.validate().unwrap();
        v.span_mm = 0.5;
        assert!(v.validate().is_err());
        v.span_mm = 100.0;
        v.depth = Some([0.0, 0.0]);
        assert!(v.validate().is_err());
        v.depth = Some([100.0, 0.0]);
        v.validate().unwrap();
        v.opacity = 1.5;
        assert!(v.validate().is_err());
        v.opacity = 0.5;
        v.color = Some([0.1, 2.0, 0.0]);
        assert!(v.validate().is_err());
        v.color = None;
        v.folder = "a//b".into();
        assert!(v.validate().is_err());
        v.folder = "Plans/Level 1".into();
        v.validate().unwrap();
        v.name = "a\nb".into();
        assert!(v.validate().is_err());
        v.name = "x".repeat(129);
        assert!(v.validate().is_err());
    }

    #[test]
    fn depth_and_work_area_bound_what_a_view_shows() {
        let mut v = view(1, "Section");
        // The default plane is world XY through the origin: +Z is towards the viewer.
        v.depth = Some([500.0, 200.0]);
        assert!(v.contains([3.0, 4.0, 499.0]));
        assert!(!v.contains([3.0, 4.0, 501.0]));
        assert!(v.contains([3.0, 4.0, -199.0]));
        assert!(!v.contains([3.0, 4.0, -201.0]));
        let clip = v.clipping().unwrap();
        assert_eq!(clip.planes.len(), 2);
        // The slab planes keep exactly the points `contains` keeps.
        for z in [-250.0, -150.0, 0.0, 450.0, 550.0] {
            let inside = clip
                .planes
                .iter()
                .all(|p| p.distance([50.0, 50.0, z]) >= 0.0);
            assert_eq!(inside, v.contains([50.0, 50.0, z]), "z = {z}");
        }
        v.work_area = Some(WorkArea::new([0.0; 3], [100.0; 3]).unwrap());
        assert!(!v.contains([-1.0, 0.0, 0.0]));
        assert!(v.contains([50.0, 50.0, 50.0]));
        assert_eq!(v.clipping().unwrap().effective().unwrap().len(), 8);
        v.opacity = 0.0;
        assert!(!v.contains([50.0, 50.0, 50.0]));
        assert!(
            view(2, "All")
                .clipping()
                .unwrap()
                .effective()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn camera_rotation_leaves_the_input_plane() {
        let mut v = view(1, "Turned");
        v.rotation_rad = [std::f64::consts::FRAC_PI_2, 0.0];
        let camera = v.camera_plane().unwrap();
        assert!((camera.x()[1] - 1.0).abs() < 1e-12);
        assert_eq!(v.plane, WorkPlane::default());
    }

    #[test]
    fn the_book_keeps_names_identities_and_folders_consistent() {
        let mut book = ViewBook::new();
        let mut a = view(1, "Plan");
        a.folder = "Plans/Ground".into();
        book.insert(a.clone()).unwrap();
        assert_eq!(
            book.folders().collect::<Vec<_>>(),
            ["Plans", "Plans/Ground"]
        );
        assert!(
            book.insert(view(2, "plan")).is_err(),
            "names are case-insensitive"
        );
        assert!(
            book.insert(view(1, "Other")).is_err(),
            "identities are unique"
        );
        book.insert(view(2, "Section")).unwrap();
        assert_eq!(book.find("SECTION").unwrap().id, id(2));
        assert_eq!(book.find(&id(1).to_string()).unwrap().name, "Plan");
        let mut renamed = view(2, "Plan");
        assert!(book.update(renamed.clone()).is_err());
        renamed.name = "Section A".into();
        book.update(renamed).unwrap();
        assert!(book.update(view(9, "Ghost")).is_err());
        assert!(
            book.delete_folder("Plans").is_err(),
            "the folder holds a view"
        );
        book.create_folder("Details").unwrap();
        assert!(book.create_folder("Details").is_err());
        book.delete_folder("Details").unwrap();
        assert!(book.remove(&id(1)).is_some());
        book.delete_folder("Plans").unwrap();
        assert_eq!(book.folders().count(), 0);
        assert_eq!(book.unique_name("Section A", &[]), "Section A (2)");
        assert_eq!(
            book.unique_name("Grid A", &["grid a".to_owned()]),
            "Grid A (2)"
        );
    }

    #[test]
    fn the_book_is_bounded_and_batches_are_atomic() {
        let mut book = ViewBook::new();
        let batch: Vec<ModelView> = (0..MAX_VIEWS as u64)
            .map(|n| view(n + 1, &format!("v{n}")))
            .collect();
        book.insert_all(batch).unwrap();
        assert!(book.insert(view(9999, "over")).is_err());
        let mut small = ViewBook::new();
        let refused = small.insert_all(vec![view(1, "a"), view(2, "A")]);
        assert!(refused.is_err());
        assert!(small.is_empty(), "a refused batch adds nothing");
    }

    #[test]
    fn folders_are_paths_not_files() {
        validate_folder("").unwrap();
        validate_folder("A/B/C").unwrap();
        for bad in ["/A", "A/", "A/../B", " A", "A/./B", "1/2/3/4/5/6/7/8/9"] {
            assert!(validate_folder(bad).is_err(), "{bad}");
        }
        assert_eq!(folder_ancestors("A/B"), ["A", "A/B"]);
        assert!(folder_within("A/B", "A"));
        assert!(!folder_within("AB", "A"));
    }

    #[test]
    fn plane_constructors_keep_the_points() {
        let p = two_points([0.0; 3], [1000.0, 0.0, 0.0]).unwrap();
        assert_eq!(p.y(), [0.0, 0.0, 1.0]);
        assert!(two_points([1.0; 3], [1.0; 3]).is_err());
        let q = three_points([0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]).unwrap();
        assert!((q.normal()[1] + 1.0).abs() < 1e-12);
        let tilted =
            two_points_in_plane([0.0; 3], [1000.0, 0.0, 500.0], WorkPlane::default()).unwrap();
        assert_eq!(tilted.x(), [1.0, 0.0, 0.0]);
        assert_eq!(tilted.y(), [0.0, 0.0, 1.0]);
        let orbit = orbit_plane([0.0; 3], 0.0, 0.0).unwrap();
        assert!(
            (orbit.normal()[0] - 1.0).abs() < 1e-12,
            "{:?}",
            orbit.normal()
        );
    }

    #[test]
    fn grid_batches_pick_by_family() {
        let grid = grid_a();
        let all = grid_views(&grid, AxisPick::All, AxisPick::All).unwrap();
        assert_eq!(
            all.iter().map(|a| a.label.as_str()).collect::<Vec<_>>(),
            ["A", "B", "1", "2"]
        );
        let ends = grid_views(&grid, AxisPick::Last, AxisPick::First).unwrap();
        assert_eq!(
            ends.iter().map(|a| a.label.as_str()).collect::<Vec<_>>(),
            ["B", "1"]
        );
        assert!(
            grid_views(&grid, AxisPick::None, AxisPick::None)
                .unwrap()
                .is_empty()
        );
        // Axis A is local x = 0, world x = 500: the plane passes through it.
        let a = grid_axis(&grid, "A").unwrap();
        assert!((a.origin()[0] - 500.0).abs() < 1e-9);
        assert!(grid_axis(&grid, "Z").is_err());
        assert!(grid_axis(&grid, &eid(2).to_string()).is_ok());
    }

    #[test]
    fn a_drawn_axis_gives_a_plane_like_a_regular_one() {
        let mut grid = grid_a();
        grid.u = GridFamily::Drawn(vec![DrawnGridLine {
            axis_id: eid(777),
            label: "C".into(),
            start_mm: [1000.0, 2000.0],
            end_mm: [1000.0, 9000.0],
        }]);
        let plane = grid_axis(&grid, "C").unwrap();
        assert!((plane.origin()[0] - 1500.0).abs() < 1e-9);
        assert_eq!(plane.y(), [0.0, 0.0, 1.0]);
    }

    #[test]
    fn level_batches_and_lookups() {
        let levels = vec![
            LevelChoice {
                id: eid(10),
                name: "Roof".into(),
                elevation_mm: 9000.0,
            },
            LevelChoice {
                id: eid(11),
                name: "Ground".into(),
                elevation_mm: 0.0,
            },
            LevelChoice {
                id: eid(12),
                name: "First".into(),
                elevation_mm: 4500.0,
            },
        ];
        let names = |pick| {
            level_batch(&levels, pick)
                .into_iter()
                .map(|l| l.name.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(AxisPick::All), ["Ground", "First", "Roof"]);
        assert_eq!(names(AxisPick::First), ["Ground"]);
        assert_eq!(names(AxisPick::Last), ["Roof"]);
        let roof = find_level(&levels, "Roof").unwrap();
        assert_eq!(level_plane(roof).unwrap().origin(), [0.0, 0.0, 9000.0]);
        assert!(find_level(&levels, "Basement").is_err());
    }

    #[test]
    fn tokens_round_trip() {
        for p in [Projection::Orthographic, Projection::Perspective] {
            assert_eq!(Projection::parse(p.as_str()).unwrap(), p);
        }
        for s in [PartSide::Back, PartSide::Bottom, PartSide::Spatial] {
            assert_eq!(PartSide::parse(s.as_str()).unwrap(), s);
        }
        for a in [
            AxisPick::None,
            AxisPick::First,
            AxisPick::Last,
            AxisPick::All,
        ] {
            assert_eq!(AxisPick::parse(a.as_str()).unwrap(), a);
        }
        assert!(PartSide::parse("left").is_err());
    }
}
