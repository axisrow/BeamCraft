// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The view panes of the work area: each one shows a model view (or the free 3D view) through
//! its own camera, in its own render mode, on its own offscreen target. Display state only: a
//! pane never touches the loaded snapshot, and the model is never written.

use crate::camera::Camera;
use beamcraft_engine::views::ModelView;
use beamcraft_engine::{KindFilter, RenderMode};
use beamcraft_model::id::Uuid7;
use egui::{Rect, Vec2, pos2, vec2};

/// Height of a pane's title bar, points.
pub const TITLE_HEIGHT: f32 = 24.0;
/// Width of a pane's resize handles along its edges, points.
pub const HANDLE: f32 = 6.0;
/// The smallest pane a move or resize leaves.
pub const MIN_PANE: Vec2 = vec2(220.0, 160.0);

/// A render target that is made on first use: the offscreen renderer and the egui texture that
/// shows it.
pub struct RenderTarget {
    renderer: Option<crate::render::Renderer>,
    texture: Option<egui::TextureId>,
}

impl RenderTarget {
    /// A target with nothing built yet.
    pub(crate) fn empty() -> Self {
        Self {
            renderer: None,
            texture: None,
        }
    }

    /// A target built already: a renderer and the egui texture registered for it.
    pub(crate) fn adopt(renderer: crate::render::Renderer, texture: egui::TextureId) -> Self {
        Self {
            renderer: Some(renderer),
            texture: Some(texture),
        }
    }

    /// The built size in pixels, `None` before the first build.
    pub(crate) fn size(&self) -> Option<[u32; 2]> {
        self.renderer.as_ref().map(|r| [r.width, r.height])
    }

    /// Resizes or creates the offscreen targets, keeping the egui texture in step. Returns the
    /// aspect of the drawn image. A zero size is refused. Records no timing: the caller decides
    /// what a resize counts as.
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
        Ok(self.aspect())
    }

    /// The renderer, once built.
    pub(crate) fn renderer(&self) -> Option<&crate::render::Renderer> {
        self.renderer.as_ref()
    }

    /// The renderer for background and ghost writes, once built.
    pub(crate) fn renderer_mut(&mut self) -> Option<&mut crate::render::Renderer> {
        self.renderer.as_mut()
    }

    /// The egui texture of the drawn image, once built.
    pub(crate) fn texture(&self) -> Option<egui::TextureId> {
        self.texture
    }

    /// Width over height of the built targets, clamped to 0.05..=20; 1.6 before the first build
    /// (the aspect a new camera starts with).
    pub(crate) fn aspect(&self) -> f32 {
        self.renderer.as_ref().map_or(1.6, |r| {
            (r.width as f32 / r.height as f32).clamp(0.05, 20.0)
        })
    }

    /// Frees the egui texture; freeing twice is harmless.
    pub(crate) fn release(&mut self, state: &eframe::egui_wgpu::RenderState) {
        if let Some(texture) = self.texture.take() {
            state.renderer.write().free_texture(&texture);
        }
    }
}

/// One view pane of the work area: a model view or the free 3D view through its own camera, on
/// its own render target. The work area's `Vec` order is the z-order; the last pane is active.
pub struct ViewPane {
    /// Stable identity for egui ids; never reused within a window.
    pub(crate) key: u64,
    /// The view it shows; `None` is the free 3D view.
    pub(crate) view: Option<Uuid7>,
    /// Its own navigation camera.
    pub(crate) camera: Camera,
    /// Its own render mode, Selected only included.
    pub(crate) mode: RenderMode,
    /// Whether its grid guides show.
    pub(crate) grids: bool,
    /// The selected-only draw and the (selected id, scene origin) it was built for, so the one
    /// engine query runs on a selection or reload change, not per frame.
    pub(crate) selected_only: ((String, [f64; 3]), Option<crate::scene::SelectedDraw>),
    /// The camera Ctrl+P returns to.
    pub(crate) toggled: Option<Camera>,
    /// The free 3D camera and display, kept while this pane shows a view.
    pub(crate) spatial: Option<(Camera, RenderMode, bool)>,
    /// The scene framing the camera was last fitted to (centre, extent, aspect): a reload that
    /// reports the same framing keeps the user's view instead of fitting again.
    pub(crate) framed: Option<([f32; 3], f32, f32)>,
    /// The image rect drawn last frame, in points; 1280 x 800 before the first frame.
    pub(crate) body: egui::Rect,
    /// The title: the view's name, or "3D view"; rewritten whenever `view` changes.
    pub(crate) title: String,
    /// Its offscreen target.
    pub(crate) target: RenderTarget,
    /// The whole window in the work area, in points.
    pub(crate) rect: Rect,
    /// False until a work-area frame has given the pane its rect.
    pub(crate) placed: bool,
    /// Whether the pane fills the work area.
    pub(crate) maximized: bool,
    /// The rect a maximized pane returns to.
    pub(crate) restore: Option<Rect>,
    /// The scene of this pane's hidden kinds when they differ from the loaded scene's, keyed by
    /// the snapshot token and the kinds it was built for.
    pub(crate) filtered: Option<(String, KindFilter, crate::render::Uploaded)>,
}

impl ViewPane {
    /// A pane on `view` (or the free 3D view) with an empty target; needs no GPU. A view brings
    /// its render mode and grid guides; the free 3D view starts shaded with grid guides on.
    pub(crate) fn new(view: Option<&ModelView>, camera: Camera, key: u64) -> Self {
        let mut pane = Self {
            key,
            view: None,
            camera,
            mode: view.map_or(RenderMode::default(), |v| v.render_mode),
            grids: view.is_none_or(|v| v.grids_visible),
            selected_only: ((String::new(), [0.0; 3]), None),
            toggled: None,
            spatial: None,
            framed: None,
            body: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0)),
            title: String::new(),
            target: RenderTarget::empty(),
            rect: Rect::NOTHING,
            placed: false,
            maximized: false,
            restore: None,
            filtered: None,
        };
        pane.set_view(view);
        pane
    }

    /// Shows `view` (or the free 3D view) and rewrites the title; the only writer of `view`
    /// outside tests.
    pub(crate) fn set_view(&mut self, view: Option<&ModelView>) {
        self.view = view.map(|v| v.id);
        self.title = view.map_or_else(|| "3D view".to_owned(), |v| v.name.clone());
    }
}

/// The parts of a pane's window, all inside `rect`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chrome {
    /// The title bar along the top.
    pub title: Rect,
    /// The image under the title bar.
    pub body: Rect,
    /// The close button at the title's right end.
    pub close: Rect,
    /// The maximize / restore button left of it.
    pub maximize: Rect,
    /// The left resize handle.
    pub left: Rect,
    /// The right resize handle.
    pub right: Rect,
    /// The top resize handle (above the title text).
    pub top: Rect,
    /// The bottom resize handle.
    pub bottom: Rect,
    /// The bottom-right corner grip.
    pub corner: Rect,
}

/// The chrome of a pane whose window is `rect`.
pub fn chrome(rect: Rect) -> Chrome {
    let bar = TITLE_HEIGHT.min(rect.height().max(0.0));
    let title = Rect::from_min_size(rect.min, vec2(rect.width().max(0.0), bar));
    let body = Rect::from_min_max(pos2(rect.min.x, title.max.y), rect.max);
    let close = Rect::from_min_max(
        pos2(rect.max.x - bar, rect.min.y),
        pos2(rect.max.x, title.max.y),
    )
    .intersect(rect);
    let maximize = close.translate(vec2(-bar, 0.0)).intersect(rect);
    let left = Rect::from_min_max(rect.min, pos2(rect.min.x + HANDLE, rect.max.y)).intersect(rect);
    let right = Rect::from_min_max(pos2(rect.max.x - HANDLE, rect.min.y), rect.max).intersect(rect);
    let top =
        Rect::from_min_max(rect.min, pos2(rect.max.x, rect.min.y + HANDLE / 2.0)).intersect(rect);
    let bottom =
        Rect::from_min_max(pos2(rect.min.x, rect.max.y - HANDLE), rect.max).intersect(rect);
    let corner =
        Rect::from_min_max(rect.max - vec2(2.0 * HANDLE, 2.0 * HANDLE), rect.max).intersect(rect);
    Chrome {
        title,
        body,
        close,
        maximize,
        left,
        right,
        top,
        bottom,
        corner,
    }
}

/// Which edge a resize drag holds; part of the handle's egui id, hence `Hash`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Edge {
    /// The left edge.
    Left,
    /// The right edge.
    Right,
    /// The top edge.
    Top,
    /// The bottom edge.
    Bottom,
    /// The bottom-right corner: both the right and the bottom edge.
    Corner,
}

/// `rect` kept fully inside `area` and at least [`MIN_PANE`] (or `area` when it is smaller).
/// Non-finite input returns `area`.
pub fn clamp_into(area: Rect, rect: Rect) -> Rect {
    if !area.is_finite() || !rect.is_finite() {
        return area;
    }
    let width = rect.width().max(MIN_PANE.x).min(area.width());
    let height = rect.height().max(MIN_PANE.y).min(area.height());
    // `min` before `max`: an area narrower than the pane keeps the pane at its left edge.
    let x = rect.min.x.min(area.max.x - width).max(area.min.x);
    let y = rect.min.y.min(area.max.y - height).max(area.min.y);
    Rect::from_min_size(pos2(x, y), vec2(width, height))
}

/// `rect` moved by `delta`, then kept inside `area`.
pub fn moved(area: Rect, rect: Rect, delta: Vec2) -> Rect {
    if !delta.is_finite() {
        return clamp_into(area, rect);
    }
    clamp_into(area, rect.translate(delta))
}

/// `rect` with `edge` dragged by `delta`; the opposite edge stays and [`MIN_PANE`] is kept.
pub fn resized(area: Rect, rect: Rect, edge: Edge, delta: Vec2) -> Rect {
    if !delta.is_finite() {
        return clamp_into(area, rect);
    }
    let mut r = rect;
    if matches!(edge, Edge::Left) {
        r.min.x = (r.min.x + delta.x)
            .min(r.max.x - MIN_PANE.x)
            .max(area.min.x);
    }
    if matches!(edge, Edge::Right | Edge::Corner) {
        r.max.x = (r.max.x + delta.x)
            .max(r.min.x + MIN_PANE.x)
            .min(area.max.x);
    }
    if matches!(edge, Edge::Top) {
        r.min.y = (r.min.y + delta.y)
            .min(r.max.y - MIN_PANE.y)
            .max(area.min.y);
    }
    if matches!(edge, Edge::Bottom | Edge::Corner) {
        r.max.y = (r.max.y + delta.y)
            .max(r.min.y + MIN_PANE.y)
            .min(area.max.y);
    }
    clamp_into(area, r)
}

/// Where the `open`-th pane opens: 2/3 of the area, offset by `TITLE_HEIGHT + 4` per pane
/// already open, wrapping before a step would leave the area.
pub fn opening_rect(area: Rect, open: usize) -> Rect {
    if !area.is_finite() {
        return area;
    }
    let size = vec2(
        (area.width() * 2.0 / 3.0).max(MIN_PANE.x).min(area.width()),
        (area.height() * 2.0 / 3.0)
            .max(MIN_PANE.y)
            .min(area.height()),
    );
    let step = TITLE_HEIGHT + 4.0;
    let room = (area.width() - size.x).min(area.height() - size.y).max(0.0);
    // Room for `steps` offsets (the first at the corner); at least one, at most a few hundred.
    let steps = ((room / step).floor() as usize)
        .saturating_add(1)
        .clamp(1, 512);
    let offset = (open % steps) as f32 * step;
    clamp_into(
        area,
        Rect::from_min_size(area.min + vec2(offset, offset), size),
    )
}

/// The key of the last pane in z-order whose rect contains `pos`.
pub fn topmost_at(panes: &[ViewPane], pos: egui::Pos2) -> Option<u64> {
    panes
        .iter()
        .rev()
        .find(|p| p.placed && p.rect.contains(pos))
        .map(|p| p.key)
}

/// Whether `area` can be laid out: finite and of non-zero size.
fn usable(area: Rect) -> bool {
    area.is_finite() && area.width() > 0.0 && area.height() > 0.0
}

/// The `k`-th of `n` equal divisions of `lo..hi`; the last one is `hi` itself, so neighbours
/// share their edge exactly and the parts add up to the whole.
fn split(lo: f32, hi: f32, k: usize, n: usize) -> f32 {
    if k >= n {
        hi
    } else {
        lo + (hi - lo) * k as f32 / n.max(1) as f32
    }
}

/// The rects of `count` panes cascaded over `area` in z-order (bottom first, so the active pane
/// ends up front-most): each 70% of the area (at least [`MIN_PANE`], at most the area), stepped
/// by `TITLE_HEIGHT + 4` down and right, wrapping to the corner when a step would leave the area.
pub fn cascade(area: Rect, count: usize) -> Vec<Rect> {
    if !usable(area) {
        return vec![area; count];
    }
    let size = vec2(
        (area.width() * 0.7).max(MIN_PANE.x).min(area.width()),
        (area.height() * 0.7).max(MIN_PANE.y).min(area.height()),
    );
    let step = TITLE_HEIGHT + 4.0;
    let room = (area.width() - size.x).min(area.height() - size.y).max(0.0);
    // Room for `steps` offsets (the first at the corner); at least one, at most a few hundred.
    let steps = ((room / step).floor() as usize)
        .saturating_add(1)
        .clamp(1, 512);
    (0..count)
        .map(|index| {
            let offset = (index % steps) as f32 * step;
            Rect::from_min_size(area.min + vec2(offset, offset), size).intersect(area)
        })
        .collect()
}

/// Stacked full-width strips; when a strip would be lower than `MIN_PANE.y` the panes form a
/// grid of `rows = floor(h / MIN_PANE.y).max(1)` rows filled column by column, the last
/// column's panes sharing its height. The rects partition the area exactly.
pub fn tile_horizontal(area: Rect, count: usize) -> Vec<Rect> {
    if !usable(area) {
        return vec![area; count];
    }
    let fits = area.height() / count.max(1) as f32 >= MIN_PANE.y;
    let rows = if fits {
        count.max(1)
    } else {
        ((area.height() / MIN_PANE.y).floor() as usize).clamp(1, count.max(1))
    };
    let columns = count.div_ceil(rows).max(1);
    (0..count)
        .map(|index| {
            let column = index / rows;
            let row = index % rows;
            // The last column holds what is left over and shares its height among those.
            let in_column = count.saturating_sub(column * rows).min(rows).max(1);
            Rect::from_min_max(
                pos2(
                    split(area.min.x, area.max.x, column, columns),
                    split(area.min.y, area.max.y, row, in_column),
                ),
                pos2(
                    split(area.min.x, area.max.x, column + 1, columns),
                    split(area.min.y, area.max.y, row + 1, in_column),
                ),
            )
        })
        .collect()
}

/// Side-by-side full-height columns; when a column would be narrower than `MIN_PANE.x`, a grid
/// of `columns = floor(w / MIN_PANE.x).max(1)` filled row by row, the last row's panes sharing
/// its width. The rects partition the area exactly.
pub fn tile_vertical(area: Rect, count: usize) -> Vec<Rect> {
    if !usable(area) {
        return vec![area; count];
    }
    let fits = area.width() / count.max(1) as f32 >= MIN_PANE.x;
    let columns = if fits {
        count.max(1)
    } else {
        ((area.width() / MIN_PANE.x).floor() as usize).clamp(1, count.max(1))
    };
    fill_rows(area, count, columns)
}

/// A square-ish grid: `ceil(sqrt(count))` columns (so four panes form 2 x 2), fewer when a
/// column would be narrower than `MIN_PANE.x`. Filled row by row; the last row's panes share
/// its width. The rects partition the area exactly.
pub fn tile_grid(area: Rect, count: usize) -> Vec<Rect> {
    if !usable(area) {
        return vec![area; count];
    }
    let mut columns = 1;
    while columns * columns < count {
        columns += 1;
    }
    let room = ((area.width() / MIN_PANE.x).floor() as usize).max(1);
    fill_rows(area, count, columns.min(room).clamp(1, count.max(1)))
}

/// `count` panes in rows of `columns`, filled row by row; the last row's panes share its width.
fn fill_rows(area: Rect, count: usize, columns: usize) -> Vec<Rect> {
    let rows = count.div_ceil(columns).max(1);
    (0..count)
        .map(|index| {
            let row = index / columns;
            let column = index % columns;
            // The last row holds what is left over and shares its width among those.
            let in_row = count.saturating_sub(row * columns).min(columns).max(1);
            Rect::from_min_max(
                pos2(
                    split(area.min.x, area.max.x, column, in_row),
                    split(area.min.y, area.max.y, row, rows),
                ),
                pos2(
                    split(area.min.x, area.max.x, column + 1, in_row),
                    split(area.min.y, area.max.y, row + 1, rows),
                ),
            )
        })
        .collect()
}

/// The whole area.
pub fn maximized(area: Rect) -> Rect {
    area
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn area() -> Rect {
        Rect::from_min_size(pos2(200.0, 150.0), vec2(1000.0, 600.0))
    }

    fn pane(key: u64, rect: Rect) -> ViewPane {
        let camera = Camera::new([0.0; 3], 1000.0, 1.6).unwrap();
        let mut pane = ViewPane::new(None, camera, key);
        pane.rect = rect;
        pane.placed = true;
        pane
    }

    #[test]
    fn the_topmost_pane_under_the_pointer_takes_it() {
        let a = pane(1, Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0)));
        let b = pane(
            2,
            Rect::from_min_size(pos2(200.0, 100.0), vec2(400.0, 300.0)),
        );
        let panes = [a, b];
        assert_eq!(topmost_at(&panes, pos2(300.0, 200.0)), Some(2));
        assert_eq!(topmost_at(&panes, pos2(50.0, 50.0)), Some(1));
        assert_eq!(topmost_at(&panes, pos2(900.0, 900.0)), None);
        // A pane not placed yet takes nothing.
        let mut hidden = pane(3, Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 900.0)));
        hidden.placed = false;
        let panes = [hidden];
        assert_eq!(topmost_at(&panes, pos2(50.0, 50.0)), None);
    }

    #[test]
    fn clamped_panes_stay_inside_and_keep_the_minimum_size() {
        let area = area();
        for rect in [
            Rect::from_min_size(pos2(-500.0, -500.0), vec2(100.0, 50.0)),
            Rect::from_min_size(pos2(1100.0, 700.0), vec2(600.0, 400.0)),
            Rect::from_min_size(pos2(0.0, 0.0), vec2(5000.0, 5000.0)),
            Rect::from_min_size(pos2(300.0, 200.0), vec2(400.0, 300.0)),
        ] {
            let clamped = clamp_into(area, rect);
            assert!(area.contains_rect(clamped), "{rect:?} -> {clamped:?}");
            assert!(clamped.width() >= MIN_PANE.x && clamped.height() >= MIN_PANE.y);
        }
        let nan = Rect::from_min_size(pos2(f32::NAN, 0.0), vec2(100.0, 100.0));
        assert_eq!(clamp_into(area, nan), area);
        let infinite = Rect::from_min_max(pos2(0.0, 0.0), pos2(f32::INFINITY, 10.0));
        assert_eq!(clamp_into(area, infinite), area);
        // An area smaller than the minimum gives the whole area.
        let small = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 80.0));
        assert_eq!(clamp_into(small, area), small);
        // A move keeps the size and stays inside.
        let moved = moved(
            area,
            Rect::from_min_size(area.min, MIN_PANE),
            vec2(5000.0, -90.0),
        );
        assert!(area.contains_rect(moved));
        assert_eq!(moved.size(), MIN_PANE);
        assert_eq!(moved.max.x, area.max.x);
    }

    #[test]
    fn resizing_never_goes_below_the_minimum() {
        let area = area();
        let rect = Rect::from_min_size(pos2(400.0, 300.0), vec2(400.0, 300.0));
        for edge in [
            Edge::Left,
            Edge::Right,
            Edge::Top,
            Edge::Bottom,
            Edge::Corner,
        ] {
            for delta in [
                vec2(-5000.0, -5000.0),
                vec2(5000.0, 5000.0),
                vec2(-150.0, 150.0),
                vec2(f32::NAN, 0.0),
            ] {
                let r = resized(area, rect, edge, delta);
                assert!(
                    r.width() >= MIN_PANE.x && r.height() >= MIN_PANE.y,
                    "{edge:?} {delta:?} -> {r:?}"
                );
                assert!(area.contains_rect(r), "{edge:?} {delta:?} -> {r:?}");
            }
        }
        // The opposite edge stays.
        let wider = resized(area, rect, Edge::Right, vec2(50.0, 0.0));
        assert_eq!((wider.min, wider.max.x), (rect.min, rect.max.x + 50.0));
        let taller = resized(area, rect, Edge::Top, vec2(0.0, -40.0));
        assert_eq!((taller.max, taller.min.y), (rect.max, rect.min.y - 40.0));
    }

    #[test]
    fn opening_rects_stay_inside_the_area() {
        for area in [
            area(),
            Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 200.0)),
            Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 100.0)),
        ] {
            for open in 0..=8 {
                let r = opening_rect(area, open);
                assert!(area.contains_rect(r), "{area:?} {open} -> {r:?}");
            }
        }
        let area = area();
        let first = opening_rect(area, 0);
        assert_eq!(first.min, area.min);
        assert_eq!(
            opening_rect(area, 1).min,
            area.min + vec2(TITLE_HEIGHT + 4.0, TITLE_HEIGHT + 4.0)
        );
    }

    #[test]
    fn the_chrome_sits_inside_the_window_with_the_body_under_the_title() {
        let rect = Rect::from_min_size(pos2(100.0, 50.0), vec2(480.0, 320.0));
        let c = chrome(rect);
        for part in [
            c.title, c.body, c.close, c.maximize, c.left, c.right, c.top, c.bottom, c.corner,
        ] {
            assert!(rect.contains_rect(part), "{part:?}");
        }
        assert_eq!(c.title.height(), TITLE_HEIGHT);
        assert_eq!(c.body.min.y, c.title.max.y);
        assert_eq!(c.body.max, rect.max);
        assert!(c.title.contains_rect(c.close) && c.title.contains_rect(c.maximize));
        assert!(c.maximize.max.x <= c.close.min.x);
    }

    /// The two areas every layout is checked on.
    fn layout_areas() -> [Rect; 2] {
        [
            Rect::from_min_size(pos2(200.0, 150.0), vec2(1200.0, 700.0)),
            Rect::from_min_size(pos2(10.0, 20.0), vec2(300.0, 200.0)),
        ]
    }

    /// Asserts that `rects` lie inside `area`, do not overlap and cover it exactly.
    fn assert_partition(area: Rect, rects: &[Rect], what: &str) {
        for r in rects {
            assert!(area.contains_rect(*r), "{what}: {r:?} outside {area:?}");
        }
        for (i, a) in rects.iter().enumerate() {
            for b in rects.iter().skip(i + 1) {
                let both = a.intersect(*b);
                let overlap = both.width().max(0.0) * both.height().max(0.0);
                assert!(overlap < 1e-3, "{what}: {a:?} overlaps {b:?}");
            }
        }
        let sum: f64 = rects
            .iter()
            .map(|r| f64::from(r.width()) * f64::from(r.height()))
            .sum();
        let whole = f64::from(area.width()) * f64::from(area.height());
        assert!((sum - whole).abs() < 1e-3, "{what}: {sum} != {whole}");
    }

    #[test]
    fn tiles_partition_the_area_for_every_count() {
        for area in layout_areas() {
            for count in 1..=8 {
                let horizontal = tile_horizontal(area, count);
                let vertical = tile_vertical(area, count);
                assert_eq!((horizontal.len(), vertical.len()), (count, count));
                assert_partition(area, &horizontal, &format!("horizontal {count} {area:?}"));
                assert_partition(area, &vertical, &format!("vertical {count} {area:?}"));
            }
        }
        // Three strips stacked in y, three columns side by side in x.
        let area = layout_areas()[0];
        let strips = tile_horizontal(area, 3);
        for (r, next) in strips.iter().zip(strips.iter().skip(1)) {
            assert_eq!((r.min.x, r.max.x), (area.min.x, area.max.x));
            assert_eq!(r.max.y, next.min.y);
        }
        let columns = tile_vertical(area, 3);
        for (r, next) in columns.iter().zip(columns.iter().skip(1)) {
            assert_eq!((r.min.y, r.max.y), (area.min.y, area.max.y));
            assert_eq!(r.max.x, next.min.x);
        }
        // Too narrow for two columns: one column of full-width panes.
        let small = layout_areas()[1];
        for r in tile_vertical(small, 8) {
            assert_eq!((r.min.x, r.max.x), (small.min.x, small.max.x));
        }
        // Too low for eight strips: a grid whose panes keep the minimum height.
        let grid = tile_horizontal(area, 8);
        assert!(grid.iter().all(|r| r.height() >= MIN_PANE.y), "{grid:?}");
    }

    #[test]
    fn the_grid_tiles_four_panes_as_two_by_two() {
        let area = layout_areas()[0];
        for count in 1..=9 {
            assert_partition(area, &tile_grid(area, count), &format!("grid {count}"));
        }
        let four = tile_grid(area, 4);
        let columns: std::collections::BTreeSet<_> =
            four.iter().map(|r| r.min.x.to_bits()).collect();
        let rows: std::collections::BTreeSet<_> = four.iter().map(|r| r.min.y.to_bits()).collect();
        assert_eq!((columns.len(), rows.len()), (2, 2), "{four:?}");
        // Three panes: two on top, the last one spans the bottom row.
        let three = tile_grid(area, 3);
        assert_eq!(three[2].width(), area.width());
        assert_eq!(three[0].width(), area.width() / 2.0);
    }

    #[test]
    fn cascaded_panes_step_down_and_right_and_wrap() {
        let step = TITLE_HEIGHT + 4.0;
        for area in layout_areas() {
            for count in 1..=8 {
                let rects = cascade(area, count);
                assert_eq!(rects.len(), count);
                for r in &rects {
                    assert!(area.contains_rect(*r), "{count} {area:?}: {r:?}");
                    assert!(r.width() >= MIN_PANE.x.min(area.width()));
                    assert!(r.height() >= MIN_PANE.y.min(area.height()));
                }
                assert_eq!(rects[0].min, area.min);
            }
            let rects = cascade(area, 30);
            // Each pane steps from the one before, or wraps back to the corner.
            for (r, next) in rects.iter().zip(rects.iter().skip(1)) {
                let stepped = next.min == r.min + vec2(step, step);
                assert!(stepped || next.min == area.min, "{r:?} -> {next:?}");
            }
            assert!(
                rects.iter().skip(1).any(|r| r.min == area.min),
                "30 panes wrap in {area:?}"
            );
        }
        // On 1200 x 700 eight panes fit without wrapping.
        let area = layout_areas()[0];
        let rects = cascade(area, 8);
        assert_eq!(rects[7].min, area.min + vec2(7.0 * step, 7.0 * step));
    }

    #[test]
    fn layouts_of_nothing_or_an_unusable_area() {
        let area = layout_areas()[0];
        assert!(cascade(area, 0).is_empty());
        assert!(tile_horizontal(area, 0).is_empty());
        assert!(tile_vertical(area, 0).is_empty());
        assert_eq!(maximized(area), area);
        let flat = Rect::from_min_size(pos2(0.0, 0.0), vec2(500.0, 0.0));
        let nan = Rect::from_min_size(pos2(f32::NAN, 0.0), vec2(100.0, 100.0));
        for bad in [flat, nan] {
            for rects in [
                cascade(bad, 3),
                tile_horizontal(bad, 3),
                tile_vertical(bad, 3),
            ] {
                assert_eq!(rects.len(), 3);
                assert!(rects.iter().all(|r| r.min.x.is_nan() || *r == bad));
            }
        }
    }
}
