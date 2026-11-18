// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Native desktop surfaces: cool neutrals, thin separators, compact Inter typography and cyan
//! reserved for actions and selection. Both palettes follow the user's appearance preference.
//!
//! Icons are contributor-original outline glyphs, painted from convex polygons so no icon file
//! is shipped. The logo itself is shown
//! unmodified from the official `assets/icons/beamcraft-1024.png`, read at run time like the font.

use egui::{
    Color32, CornerRadius, FontFamily, FontId, Painter, Pos2, Rect, Response, RichText, Sense,
    Shadow, Stroke, Theme, Ui, Vec2, pos2, vec2,
};
use std::path::{Path, PathBuf};

/// Brand cyan (logo background).
pub const BRAND: Color32 = Color32::from_rgb(0x00, 0xB4, 0xD8);
/// Light cyan (logo grid).
pub const BRAND_LIGHT: Color32 = Color32::from_rgb(0x3F, 0xCD, 0xEB);
/// Cream (logo faces).
pub const CREAM: Color32 = Color32::from_rgb(0xF5, 0xF0, 0xE1);
/// Grey-teal (logo sides).
pub const TEAL: Color32 = Color32::from_rgb(0x78, 0xB4, 0xC4);
/// The darker teal of a side turned away from the light.
pub const TEAL_DARK: Color32 = Color32::from_rgb(0x5E, 0x9A, 0xAB);
/// Ink (logo outline), also the text on brand cyan in both themes.
pub const INK: Color32 = Color32::from_rgb(0x11, 0x11, 0x11);

/// The colours of one theme.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    /// Whether this is the dark theme.
    pub dark: bool,
    /// Window background.
    pub page: Color32,
    /// Cards, panels and fields.
    pub surface: Color32,
    /// Table heads and summary boxes.
    pub head: Color32,
    /// Dark ink for illustrations.
    pub ink: Color32,
    /// Outline of icons drawn on the page.
    pub line: Color32,
    /// Body text.
    pub text: Color32,
    /// Secondary text (at least 4.5:1 on `page` and `surface`).
    pub muted: Color32,
    /// Thin separators.
    pub rule: Color32,
    /// Input outlines, distinct from subtle panel separators.
    pub control: Color32,
    /// Side panel ground and its grid.
    pub side: Color32,
    /// Grid lines of the side panel.
    pub side_grid: Color32,
    /// A selected row.
    pub select: Color32,
    /// Links.
    pub link: Color32,
    /// Error text.
    pub error: Color32,
}

impl Palette {
    /// The palette of `theme`.
    pub const fn of(theme: Theme) -> Self {
        match theme {
            Theme::Light => Self {
                dark: false,
                page: Color32::from_rgb(0xF5, 0xF7, 0xF9),
                surface: Color32::from_rgb(0xFB, 0xFD, 0xFF),
                head: Color32::from_rgb(0xE9, 0xED, 0xF0),
                ink: INK,
                line: INK,
                text: Color32::from_rgb(0x23, 0x2D, 0x36),
                muted: Color32::from_rgb(0x5C, 0x68, 0x73),
                rule: Color32::from_rgb(0xCB, 0xD5, 0xDC),
                control: Color32::from_rgb(0x82, 0x91, 0x9C),
                side: Color32::from_rgb(0x20, 0x2A, 0x33),
                side_grid: Color32::from_rgb(0x36, 0x44, 0x4F),
                select: Color32::from_rgb(0xE7, 0xEE, 0xF2),
                link: Color32::from_rgb(0x00, 0x6F, 0x87),
                error: Color32::from_rgb(0xA8, 0x1E, 0x12),
            },
            Theme::Dark => Self {
                dark: true,
                page: Color32::from_rgb(0x14, 0x1C, 0x24),
                surface: Color32::from_rgb(0x1C, 0x26, 0x31),
                head: Color32::from_rgb(0x19, 0x23, 0x2D),
                ink: Color32::BLACK,
                line: Color32::from_rgb(0xE6, 0xEB, 0xF0),
                text: Color32::from_rgb(0xE6, 0xEB, 0xF0),
                muted: Color32::from_rgb(0xAA, 0xB5, 0xC0),
                rule: Color32::from_rgb(0x37, 0x47, 0x55),
                control: Color32::from_rgb(0x63, 0x7B, 0x8F),
                side: Color32::from_rgb(0x10, 0x18, 0x21),
                side_grid: Color32::from_rgb(0x2B, 0x3F, 0x50),
                select: Color32::from_rgb(0x2B, 0x40, 0x51),
                link: BRAND_LIGHT,
                error: Color32::from_rgb(0xFF, 0x8A, 0x7A),
            },
        }
    }

    /// The palette egui is drawing with now.
    pub fn current(ui: &Ui) -> Self {
        Self::of(ui.ctx().theme())
    }

    /// The text colour on a side panel.
    pub const fn side_text(&self) -> Color32 {
        Color32::from_rgb(0xE3, 0xEC, 0xF2)
    }
}

fn visuals(palette: Palette) -> egui::Visuals {
    let mut v = if palette.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let radius = CornerRadius::same(5);
    let border = Stroke::new(1.0, palette.rule);
    v.panel_fill = palette.page;
    v.window_fill = palette.surface;
    v.window_stroke = border;
    v.window_corner_radius = CornerRadius::same(6);
    v.window_shadow = Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(35),
    };
    v.popup_shadow = Shadow {
        offset: [0, 2],
        blur: 8,
        spread: 0,
        color: Color32::from_black_alpha(30),
    };
    v.menu_corner_radius = radius;
    v.extreme_bg_color = palette.surface;
    v.faint_bg_color = palette.head;
    v.hyperlink_color = palette.link;
    v.error_fg_color = palette.error;
    v.override_text_color = Some(palette.text);
    v.selection.bg_fill = palette.select;
    v.selection.stroke = Stroke::new(1.0, palette.text);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = palette.surface;
    w.noninteractive.weak_bg_fill = palette.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, palette.rule);
    w.noninteractive.fg_stroke = Stroke::new(1.0, palette.text);
    w.noninteractive.corner_radius = radius;
    for (state, fill) in [
        (&mut w.inactive, palette.surface),
        (&mut w.hovered, palette.head),
        (&mut w.active, palette.select),
        (&mut w.open, palette.head),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, palette.control);
        state.corner_radius = radius;
        state.expansion = 0.0;
    }
    w.inactive.fg_stroke = Stroke::new(1.5, palette.text);
    w.hovered.fg_stroke = Stroke::new(1.5, palette.text);
    w.open.fg_stroke = Stroke::new(1.5, palette.text);
    w.active.fg_stroke = Stroke::new(1.5, palette.text);
    v
}

/// Installs both themes and lets the operating system choose between them; also sizes text for
/// a desktop window (14 px body).
pub fn install(ctx: &egui::Context) {
    ctx.set_visuals_of(Theme::Light, visuals(Palette::of(Theme::Light)));
    ctx.set_visuals_of(Theme::Dark, visuals(Palette::of(Theme::Dark)));
    // BEAMCRAFT_THEME=light|dark overrides the operating system (screenshots, odd desktops).
    let preference = match std::env::var("BEAMCRAFT_THEME").as_deref() {
        Ok("light") => egui::ThemePreference::Light,
        Ok("dark") => egui::ThemePreference::Dark,
        _ => egui::ThemePreference::System,
    };
    let installed = egui::Id::unique("beamcraft-style-installed");
    if !ctx.data(|data| data.get_temp::<bool>(installed).unwrap_or(false)) {
        ctx.set_theme(preference);
        ctx.data_mut(|data| data.insert_temp(installed, true));
    }
    ctx.all_styles_mut(|style| {
        style.animation_time = 0.0;
        style.spacing.item_spacing = vec2(8.0, 6.0);
        style.spacing.button_padding = vec2(10.0, 5.0);
        style.spacing.interact_size.y = 28.0;
        style.text_styles.insert(
            egui::TextStyle::Body,
            FontId::new(13.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            FontId::new(12.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Small,
            FontId::new(11.0, FontFamily::Proportional),
        );
        style
            .text_styles
            .insert(egui::TextStyle::Heading, FontId::new(24.0, bold_family()));
    });
}

/// The family of Inter Bold when it was loaded, else the regular family.
pub fn bold_family() -> FontFamily {
    FontFamily::Name("bold".into())
}

/// Bold text of `size`.
pub fn bold(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).family(bold_family()).size(size)
}

/// A flat panel with a one-point separator.
pub fn card(palette: Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(palette.surface)
        .stroke(Stroke::new(1.0, palette.control))
        .corner_radius(CornerRadius::same(5))
}

/// How a painted button looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Brand cyan with ink text: the one main action of a view.
    Primary,
    /// Surface fill with a neutral border.
    Secondary,
}

/// A native, keyboard-accessible desktop button with a visible focus outline.
pub fn button(ui: &mut Ui, kind: Kind, text: &str, enabled: bool) -> Response {
    let palette = Palette::current(ui);
    let (fill, ink) = match kind {
        Kind::Primary => (BRAND, INK),
        Kind::Secondary => (palette.surface, palette.text),
    };
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).size(12.0).color(ink))
            .fill(fill)
            .stroke(Stroke::new(1.0, palette.rule))
            .corner_radius(5)
            .min_size(vec2(64.0, 36.0)),
    )
}

/// A labeled desktop button with a centered original line icon and text.
pub fn icon_text_button(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    enabled: bool,
    width: f32,
) -> Response {
    let palette = Palette::current(ui);
    let response = ui.add_enabled(
        enabled,
        egui::Button::new("")
            .fill(palette.surface)
            .stroke(Stroke::new(1.0, palette.rule))
            .corner_radius(5)
            .min_size(vec2(width, 36.0)),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    let mut painter = ui.painter().clone();
    if !enabled {
        painter.multiply_opacity(0.35);
    }
    let text = painter.layout_no_wrap(label.to_owned(), FontId::proportional(12.0), palette.text);
    let left = response.rect.center().x - (16.0 + 8.0 + text.size().x) / 2.0;
    paint_icon(
        &painter,
        Rect::from_min_size(pos2(left, response.rect.center().y - 8.0), vec2(16.0, 16.0)),
        icon,
        palette.text,
    );
    painter.galley(
        pos2(left + 24.0, response.rect.center().y - text.size().y / 2.0),
        text,
        palette.text,
    );
    response
}

/// The width the system's window controls take at the left of a title bar, in points. On macOS
/// the window keeps its native frame with a transparent title bar under our own header, so the
/// traffic lights sit over the start of the header row; elsewhere the system draws its own title
/// bar with its own controls above the header, and nothing is reserved.
pub const NATIVE_CONTROLS_INSET: f32 = if cfg!(target_os = "macos") { 72.0 } else { 0.0 };

/// Reserves the room the system's window controls take at the start of a title bar row.
pub fn native_controls_inset(ui: &mut Ui) {
    if NATIVE_CONTROLS_INSET > 0.0 {
        ui.add_space(NATIVE_CONTROLS_INSET);
    }
}

/// Whether a closable tab puts its close glyph first, as macOS tabs do, rather than last, as
/// Windows and Linux tabs do.
pub const TAB_CLOSE_LEADING: bool = cfg!(target_os = "macos");

/// Dragging and double-click maximization of the header row on macOS, where the header is
/// drawn under the transparent native title bar. Elsewhere the system's own title bar moves the
/// window and the header row is an ordinary toolbar.
pub fn titlebar_drag(ui: &mut Ui) {
    titlebar_drag_with_maximize(ui, true);
}

/// [`titlebar_drag`] that cannot maximize a fixed-size welcome window.
pub fn titlebar_drag_with_maximize(ui: &mut Ui, maximize: bool) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let response = ui.interact(
        ui.max_rect(),
        ui.scope_id().with("title-drag"),
        Sense::click_and_drag(),
    );
    if response.drag_started() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if maximize && response.double_clicked() {
        let maximized = ui.input(|input| input.viewport().maximized.unwrap_or(false));
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    }
}

/// A compact, keyboard-accessible original line-icon control.
pub fn icon_button(ui: &mut Ui, icon: Icon, label: &str, enabled: bool) -> Response {
    let response = ui.add_enabled(
        enabled,
        egui::Button::new("")
            .frame(false)
            .min_size(vec2(26.0, 26.0)),
    );
    let palette = Palette::current(ui);
    let mut painter = ui.painter().clone();
    if !enabled {
        painter.multiply_opacity(0.35);
    }
    paint_icon(&painter, response.rect.shrink(5.0), icon, palette.text);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    response.on_hover_text(label)
}

/// A compact icon toggle: filled with the selection colour while `active`, highlighted on hover.
pub fn icon_toggle(ui: &mut Ui, icon: Icon, label: &str, active: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    let palette = Palette::current(ui);
    if active || response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(rect, 4, if active { palette.select } else { palette.head });
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            4,
            Stroke::new(1.0, palette.link),
            egui::StrokeKind::Inside,
        );
    }
    let ink = if active { palette.text } else { palette.muted };
    paint_icon(ui.painter(), rect.shrink(5.0), icon, ink);
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, active, label));
    response.on_hover_text(label)
}

/// A compact icon toggle: the star of a favourite fills with the link colour while `active`.
pub fn icon_toggle_filled(ui: &mut Ui, icon: Icon, label: &str, active: bool) -> Response {
    let response = icon_toggle(ui, icon, label, active);
    if active {
        let palette = Palette::current(ui);
        paint_icon(ui.painter(), response.rect.shrink(5.0), icon, palette.link);
    }
    response
}

/// Paints the blueprint grid of `spacing` points over `rect`.
pub fn grid(painter: &Painter, rect: Rect, fill: Color32, line: Color32, spacing: f32) {
    painter.rect_filled(rect, 0.0, fill);
    let stroke = Stroke::new(1.5, line);
    let mut x = rect.left() + spacing;
    while x < rect.right() {
        painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], stroke);
        x += spacing;
    }
    let mut y = rect.top() + spacing;
    while y < rect.bottom() {
        painter.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], stroke);
        y += spacing;
    }
}

/// The isometric glyphs of the interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    /// A clock face: recent models.
    Recent,
    /// A folder: models of a folder.
    Folder,
    /// A shared (cloud) model.
    Shared,
    /// An isometric block: a model, the Blank environment and template.
    Block,
    /// A plate.
    Plate,
    /// An I-section beam.
    Beam,
    /// An I-section column.
    Column,
    /// Plus.
    Plus,
    /// Back arrow.
    Back,
    /// Refresh arrow.
    Refresh,
    /// The selection pointer.
    Select,
    /// A bolt: hexagon head and shank.
    Bolt,
    /// Two plates joined by a weld bead.
    Weld,
    /// Four arrows: move.
    Move,
    /// Two overlapping sheets: copy.
    Copy,
    /// A bin: delete.
    Delete,
    /// Arrow turning back: undo.
    Undo,
    /// Arrow turning forward: redo.
    Redo,
    /// An outlined block: wireframe display.
    Wire,
    /// Two families of straight construction axes.
    Grid,
    /// Four corners framing the current model.
    Fit,
    /// The square endpoint snap marker.
    End,
    /// The triangular midpoint snap marker.
    Middle,
    /// The circular nearest-point snap marker.
    Nearest,
    /// A leader callout: a dot at the feature, an angled line and a small text box.
    Callout,
    /// A view frame with a section line and a plus: create a view.
    NewView,
    /// Three cascaded window frames: arrange the view windows.
    Arrange,
    /// A view frame with a section line: a model view.
    ViewFrame,
    /// A horizontal slab seen from above with the cut level line: a plan view.
    PlanView,
    /// A vertical frame standing on a ground line: an elevation or section view.
    ElevationView,
    /// A tilted frame on a ground line: an inclined view.
    InclinedView,
    /// A funnel: filter what a list shows.
    Filter,
    /// A pencil: rename.
    Rename,
    /// A diagonal member between two points: a brace.
    Brace,
    /// A ruler with its ticks: measure.
    Measure,
    /// A box with its hidden edges dashed: the transparent display.
    Transparent,
    /// A box filled in one flat tone: the grey display.
    GreyShaded,
    /// A box with lit and shadowed faces: the shaded display.
    Shaded,
    /// A dashed box around one solid part: only the selection is drawn.
    SelectedOnly,
    /// A palette with three paint wells: colour by a colour set.
    Palette,
    /// A box with a plane slicing through it: the cut plane.
    CutPlane,
    /// Two frames side by side: open a view beside the active one.
    Beside,
    /// A frame with a cross and a frame behind it: close the other views.
    CloseOthers,
    /// A page with a folded corner and text lines: a drawing sheet.
    Sheet,
    /// A table of rows and columns: a schedule.
    Table,
    /// A five-pointed star: favourite.
    Star,
    /// A magnifier: search.
    Search,
    /// A folder with a plus: create a folder.
    NewFolder,
    /// A picture frame with a hill: view previews.
    Preview,
    /// A clipboard: paste.
    Paste,
    /// A house: the default view.
    Home,
    /// A cross: close or cancel.
    Close,
    /// A tick: confirm.
    Check,
}

#[derive(Clone, Copy)]
enum Face {
    Cream,
    Teal,
    TealDark,
}

/// One convex face on the 24 x 24 icon grid.
type Polygon = (&'static [[f32; 2]], Face);

const BLOCK: &[Polygon] = &[
    (
        &[[4.0, 7.5], [4.0, 16.5], [12.0, 21.0], [12.0, 12.0]],
        Face::TealDark,
    ),
    (
        &[[12.0, 12.0], [12.0, 21.0], [20.0, 16.5], [20.0, 7.5]],
        Face::Teal,
    ),
    (
        &[[12.0, 3.0], [20.0, 7.5], [12.0, 12.0], [4.0, 7.5]],
        Face::Cream,
    ),
];
const FOLDER: &[Polygon] = &[
    (
        &[[3.0, 5.5], [9.5, 5.5], [11.5, 8.0], [3.0, 8.0]],
        Face::Teal,
    ),
    (
        &[[3.0, 8.0], [21.0, 8.0], [21.0, 19.0], [3.0, 19.0]],
        Face::Teal,
    ),
    (
        &[[3.0, 10.5], [21.0, 10.5], [21.0, 19.0], [3.0, 19.0]],
        Face::Cream,
    ),
];

const DELETE: &[Polygon] = &[
    (
        &[[6.0, 8.0], [18.0, 8.0], [17.0, 21.0], [7.0, 21.0]],
        Face::Cream,
    ),
    (
        &[[4.0, 5.0], [20.0, 5.0], [20.0, 8.0], [4.0, 8.0]],
        Face::Teal,
    ),
];

/// Paints `icon` into `rect`; `line` outlines the faces and draws the strokes.
pub fn paint_icon(painter: &Painter, rect: Rect, icon: Icon, line: Color32) {
    let scale = rect.width().min(rect.height()) / 24.0;
    let origin = rect.center() - Vec2::splat(12.0 * scale);
    let at = |p: [f32; 2]| origin + vec2(p[0], p[1]) * scale;
    let stroke = Stroke::new((1.5 * scale).max(1.2), line);
    let faces = |polygons: &[Polygon]| {
        for (points, _) in polygons {
            painter.add(egui::Shape::convex_polygon(
                points.iter().map(|p| at(*p)).collect(),
                Color32::TRANSPARENT,
                stroke,
            ));
        }
    };
    // One outlined face with a tonal fill of the line colour (0 leaves it open), as the
    // display-mode icons are drawn.
    let shade = |points: &[[f32; 2]], tone: f32| {
        painter.add(egui::Shape::convex_polygon(
            points.iter().map(|p| at(*p)).collect(),
            if tone > 0.0 {
                stroke.color.gamma_multiply(tone)
            } else {
                Color32::TRANSPARENT
            },
            stroke,
        ));
    };
    let polyline = |points: &[[f32; 2]]| {
        painter.add(egui::Shape::line(
            points.iter().map(|p| at(*p)).collect(),
            stroke,
        ));
    };
    match icon {
        Icon::End => {
            painter.rect_stroke(
                Rect::from_min_max(at([6.0, 6.0]), at([18.0, 18.0])),
                0,
                stroke,
                egui::StrokeKind::Inside,
            );
        }
        Icon::Middle => {
            painter.add(egui::Shape::closed_line(
                [[12.0, 4.0], [21.0, 20.0], [3.0, 20.0]].map(at).to_vec(),
                stroke,
            ));
        }
        Icon::Nearest => {
            painter.circle_stroke(at([12.0, 12.0]), 8.0 * scale, stroke);
        }
        Icon::Fit => {
            for corner in [
                [[4.0, 9.0], [4.0, 4.0], [9.0, 4.0]],
                [[15.0, 4.0], [20.0, 4.0], [20.0, 9.0]],
                [[20.0, 15.0], [20.0, 20.0], [15.0, 20.0]],
                [[9.0, 20.0], [4.0, 20.0], [4.0, 15.0]],
            ] {
                polyline(&corner);
            }
        }
        Icon::Select => {
            // Star-shaped from its tip: a fan of two triangles, then the outline.
            let [a, b, c, d] = [[5.0, 3.0], [18.0, 11.0], [12.0, 12.5], [9.0, 19.0]];
            for triangle in [[a, b, c], [a, c, d]] {
                painter.add(egui::Shape::convex_polygon(
                    triangle.map(at).to_vec(),
                    Color32::TRANSPARENT,
                    Stroke::NONE,
                ));
            }
            let tip = [a, b, c, d];
            painter.add(egui::Shape::closed_line(tip.map(at).to_vec(), stroke));
        }
        Icon::Bolt => {
            // Hexagon head over a threaded shank.
            shade(
                &[
                    [8.5, 3.0],
                    [15.5, 3.0],
                    [17.5, 5.75],
                    [15.5, 8.5],
                    [8.5, 8.5],
                    [6.5, 5.75],
                ],
                0.45,
            );
            shade(
                &[[10.25, 8.5], [13.75, 8.5], [13.75, 20.5], [10.25, 20.5]],
                0.15,
            );
            for y in [12.0, 15.0, 18.0] {
                polyline(&[[10.25, y], [13.75, y - 1.0]]);
            }
        }
        Icon::Weld => {
            // Two plates at a right angle, the fillet bead in the corner.
            shade(
                &[[3.0, 15.0], [21.0, 15.0], [21.0, 19.5], [3.0, 19.5]],
                0.15,
            );
            shade(
                &[[10.0, 4.0], [14.0, 4.0], [14.0, 15.0], [10.0, 15.0]],
                0.15,
            );
            shade(&[[14.0, 15.0], [14.0, 11.0], [18.0, 15.0]], 0.65);
            shade(&[[10.0, 15.0], [10.0, 11.0], [6.0, 15.0]], 0.65);
        }
        Icon::Copy => {
            shade(&[[9.0, 3.0], [20.0, 3.0], [20.0, 15.0], [9.0, 15.0]], 0.0);
            shade(&[[4.0, 9.0], [15.0, 9.0], [15.0, 21.0], [4.0, 21.0]], 0.2);
        }
        Icon::Brace => {
            // A member running diagonally between two gusset points.
            shade(&[[3.5, 17.5], [6.5, 20.5], [20.5, 6.5], [17.5, 3.5]], 0.35);
            painter.circle_filled(at([5.0, 19.0]), 2.0 * scale, stroke.color);
            painter.circle_filled(at([19.0, 5.0]), 2.0 * scale, stroke.color);
        }
        Icon::Measure => {
            // A ruler with ticks and the measured span over it.
            shade(
                &[[3.0, 11.0], [21.0, 11.0], [21.0, 17.0], [3.0, 17.0]],
                0.15,
            );
            for (x, long) in [
                (6.0, false),
                (9.0, true),
                (12.0, false),
                (15.0, true),
                (18.0, false),
            ] {
                polyline(&[[x, 11.0], [x, if long { 14.5 } else { 13.0 }]]);
            }
            polyline(&[[3.0, 6.0], [21.0, 6.0]]);
            polyline(&[[3.0, 4.0], [3.0, 8.0]]);
            polyline(&[[21.0, 4.0], [21.0, 8.0]]);
        }
        Icon::Delete => {
            faces(DELETE);
            polyline(&[[9.0, 5.0], [9.0, 3.0], [15.0, 3.0], [15.0, 5.0]]);
        }
        Icon::Move => {
            polyline(&[[12.0, 3.0], [12.0, 21.0]]);
            polyline(&[[3.0, 12.0], [21.0, 12.0]]);
            for arrow in [
                [[9.0, 6.0], [12.0, 3.0], [15.0, 6.0]],
                [[9.0, 18.0], [12.0, 21.0], [15.0, 18.0]],
                [[6.0, 9.0], [3.0, 12.0], [6.0, 15.0]],
                [[18.0, 9.0], [21.0, 12.0], [18.0, 15.0]],
            ] {
                polyline(&arrow);
            }
        }
        Icon::Undo | Icon::Redo => {
            let flip = |p: [f32; 2]| match icon {
                Icon::Redo => [24.0 - p[0], p[1]],
                _ => p,
            };
            let arc = [
                [4.0, 9.0],
                [14.0, 9.0],
                [17.0, 9.8],
                [19.2, 12.0],
                [20.0, 15.0],
                [19.2, 18.0],
                [17.0, 20.2],
                [14.0, 21.0],
                [11.0, 21.0],
            ];
            polyline(&arc.map(flip));
            polyline(&[[9.0, 4.0], [4.0, 9.0], [9.0, 14.0]].map(flip));
        }
        Icon::Grid => {
            for offset in [6.0, 12.0, 18.0] {
                polyline(&[[offset, 3.0], [offset, 21.0]]);
                polyline(&[[3.0, offset], [21.0, offset]]);
            }
        }
        Icon::Callout => {
            painter.circle_filled(at([6.0, 18.0]), 2.2 * scale, stroke.color);
            polyline(&[[6.0, 18.0], [11.0, 13.0], [15.0, 13.0]]);
            polyline(&[
                [15.0, 8.0],
                [20.0, 8.0],
                [20.0, 16.0],
                [15.0, 16.0],
                [15.0, 8.0],
            ]);
        }
        Icon::NewView => {
            polyline(&[
                [3.0, 4.0],
                [17.0, 4.0],
                [17.0, 16.0],
                [3.0, 16.0],
                [3.0, 4.0],
            ]);
            polyline(&[[6.0, 13.0], [14.0, 7.0]]);
            polyline(&[[19.0, 15.0], [19.0, 21.0]]);
            polyline(&[[16.0, 18.0], [22.0, 18.0]]);
        }
        Icon::Arrange => {
            polyline(&[
                [2.0, 3.0],
                [13.0, 3.0],
                [13.0, 11.0],
                [2.0, 11.0],
                [2.0, 3.0],
            ]);
            polyline(&[
                [6.0, 7.0],
                [17.0, 7.0],
                [17.0, 15.0],
                [6.0, 15.0],
                [6.0, 7.0],
            ]);
            polyline(&[
                [10.0, 11.0],
                [21.0, 11.0],
                [21.0, 19.0],
                [10.0, 19.0],
                [10.0, 11.0],
            ]);
        }
        Icon::Wire => {
            painter.add(egui::Shape::closed_line(
                [
                    [12.0, 3.0],
                    [20.0, 7.5],
                    [20.0, 16.5],
                    [12.0, 21.0],
                    [4.0, 16.5],
                    [4.0, 7.5],
                ]
                .map(at)
                .to_vec(),
                stroke,
            ));
            polyline(&[[4.0, 7.5], [12.0, 12.0], [20.0, 7.5]]);
            polyline(&[[12.0, 12.0], [12.0, 21.0]]);
        }
        Icon::ViewFrame => {
            polyline(&[
                [3.0, 4.0],
                [21.0, 4.0],
                [21.0, 20.0],
                [3.0, 20.0],
                [3.0, 4.0],
            ]);
            polyline(&[[3.0, 8.0], [21.0, 8.0]]);
            polyline(&[[7.0, 17.0], [17.0, 11.0]]);
        }
        Icon::PlanView => {
            // A slab in plan: a parallelogram seen from above, with the cut level across it.
            polyline(&[
                [3.0, 15.0],
                [9.0, 9.0],
                [21.0, 9.0],
                [15.0, 15.0],
                [3.0, 15.0],
            ]);
            polyline(&[[6.0, 19.0], [18.0, 19.0]]);
            polyline(&[[12.0, 3.0], [12.0, 7.0]]);
            polyline(&[[10.0, 5.0], [12.0, 7.0], [14.0, 5.0]]);
        }
        Icon::ElevationView => {
            // A frame standing upright on a ground line, looked at from the side.
            polyline(&[[6.0, 19.0], [6.0, 5.0], [18.0, 5.0], [18.0, 19.0]]);
            polyline(&[[3.0, 19.0], [21.0, 19.0]]);
            polyline(&[[6.0, 12.0], [18.0, 12.0]]);
        }
        Icon::Transparent => {
            painter.add(egui::Shape::closed_line(
                [
                    [12.0, 3.0],
                    [20.0, 7.5],
                    [20.0, 16.5],
                    [12.0, 21.0],
                    [4.0, 16.5],
                    [4.0, 7.5],
                ]
                .map(at)
                .to_vec(),
                stroke,
            ));
            polyline(&[[4.0, 7.5], [12.0, 12.0], [20.0, 7.5]]);
            polyline(&[[12.0, 12.0], [12.0, 21.0]]);
            // The hidden edges, seen through the faces.
            painter.extend(egui::Shape::dashed_line(
                &[at([4.0, 16.5]), at([12.0, 12.5]), at([20.0, 16.5])],
                Stroke::new(stroke.width * 0.8, stroke.color),
                2.0 * scale,
                2.0 * scale,
            ));
            painter.extend(egui::Shape::dashed_line(
                &[at([12.0, 3.0]), at([12.0, 12.5])],
                Stroke::new(stroke.width * 0.8, stroke.color),
                2.0 * scale,
                2.0 * scale,
            ));
        }
        Icon::GreyShaded => {
            painter.add(egui::Shape::convex_polygon(
                [
                    [12.0, 3.0],
                    [20.0, 7.5],
                    [20.0, 16.5],
                    [12.0, 21.0],
                    [4.0, 16.5],
                    [4.0, 7.5],
                ]
                .map(at)
                .to_vec(),
                stroke.color.gamma_multiply(0.3),
                stroke,
            ));
            polyline(&[[4.0, 7.5], [12.0, 12.0], [20.0, 7.5]]);
            polyline(&[[12.0, 12.0], [12.0, 21.0]]);
        }
        Icon::Shaded => {
            let face = |points: &[[f32; 2]], tone: f32| {
                painter.add(egui::Shape::convex_polygon(
                    points.iter().map(|p| at(*p)).collect(),
                    stroke.color.gamma_multiply(tone),
                    stroke,
                ));
            };
            face(&[[12.0, 3.0], [20.0, 7.5], [12.0, 12.0], [4.0, 7.5]], 0.15);
            face(&[[4.0, 7.5], [12.0, 12.0], [12.0, 21.0], [4.0, 16.5]], 0.55);
            face(
                &[[12.0, 12.0], [20.0, 7.5], [20.0, 16.5], [12.0, 21.0]],
                0.35,
            );
        }
        Icon::SelectedOnly => {
            painter.extend(egui::Shape::dashed_line(
                &[
                    [12.0, 3.0],
                    [20.0, 7.5],
                    [20.0, 16.5],
                    [12.0, 21.0],
                    [4.0, 16.5],
                    [4.0, 7.5],
                ]
                .map(at)
                .into_iter()
                .chain([at([12.0, 3.0])])
                .collect::<Vec<_>>(),
                Stroke::new(stroke.width * 0.8, stroke.color),
                2.0 * scale,
                2.0 * scale,
            ));
            painter.add(egui::Shape::convex_polygon(
                [
                    [12.0, 8.5],
                    [16.0, 10.75],
                    [16.0, 15.25],
                    [12.0, 17.5],
                    [8.0, 15.25],
                    [8.0, 10.75],
                ]
                .map(at)
                .to_vec(),
                stroke.color.gamma_multiply(0.6),
                stroke,
            ));
        }
        Icon::Palette => {
            painter.add(egui::Shape::closed_line(
                [
                    [12.0, 3.5],
                    [19.0, 6.5],
                    [21.0, 12.5],
                    [17.5, 18.5],
                    [13.5, 17.0],
                    [12.0, 20.5],
                    [6.0, 19.0],
                    [3.0, 12.5],
                    [5.5, 6.5],
                ]
                .map(at)
                .to_vec(),
                stroke,
            ));
            for well in [[8.5, 10.0], [12.5, 7.5], [16.5, 10.5]] {
                painter.circle_filled(at(well), 1.6 * scale, stroke.color);
            }
        }
        Icon::CutPlane => {
            painter.add(egui::Shape::closed_line(
                [
                    [12.0, 3.0],
                    [20.0, 7.5],
                    [20.0, 16.5],
                    [12.0, 21.0],
                    [4.0, 16.5],
                    [4.0, 7.5],
                ]
                .map(at)
                .to_vec(),
                stroke,
            ));
            polyline(&[[4.0, 7.5], [12.0, 12.0], [20.0, 7.5]]);
            polyline(&[[12.0, 12.0], [12.0, 21.0]]);
            // The cutting plane, drawn across the box and past it.
            painter.add(egui::Shape::convex_polygon(
                [[1.5, 11.0], [12.0, 5.0], [22.5, 11.0], [12.0, 17.0]]
                    .map(at)
                    .to_vec(),
                stroke.color.gamma_multiply(0.25),
                Stroke::new(stroke.width, stroke.color),
            ));
        }
        Icon::Beside => {
            for (x0, x1) in [(3.0, 11.0), (13.0, 21.0)] {
                polyline(&[[x0, 5.0], [x1, 5.0], [x1, 19.0], [x0, 19.0], [x0, 5.0]]);
                polyline(&[[x0, 8.0], [x1, 8.0]]);
            }
        }
        Icon::CloseOthers => {
            polyline(&[[8.0, 3.0], [21.0, 3.0], [21.0, 15.0]]);
            polyline(&[
                [3.0, 7.0],
                [17.0, 7.0],
                [17.0, 21.0],
                [3.0, 21.0],
                [3.0, 7.0],
            ]);
            polyline(&[[7.0, 11.0], [13.0, 17.0]]);
            polyline(&[[13.0, 11.0], [7.0, 17.0]]);
        }
        Icon::Rename => {
            polyline(&[
                [5.0, 19.0],
                [5.5, 15.0],
                [16.0, 4.5],
                [19.5, 8.0],
                [9.0, 18.5],
                [5.0, 19.0],
            ]);
            polyline(&[[14.0, 6.5], [17.5, 10.0]]);
        }
        Icon::Filter => {
            polyline(&[
                [4.0, 5.0],
                [20.0, 5.0],
                [14.0, 12.5],
                [14.0, 19.0],
                [10.0, 17.0],
                [10.0, 12.5],
                [4.0, 5.0],
            ]);
        }
        Icon::InclinedView => {
            // A frame tilted off the vertical, standing on a ground line.
            polyline(&[[5.0, 19.0], [10.0, 5.0], [20.0, 5.0], [15.0, 19.0]]);
            polyline(&[[3.0, 19.0], [21.0, 19.0]]);
        }
        Icon::Sheet => {
            polyline(&[
                [5.0, 3.0],
                [14.0, 3.0],
                [19.0, 8.0],
                [19.0, 21.0],
                [5.0, 21.0],
                [5.0, 3.0],
            ]);
            polyline(&[[14.0, 3.0], [14.0, 8.0], [19.0, 8.0]]);
            polyline(&[[8.0, 13.0], [16.0, 13.0]]);
            polyline(&[[8.0, 17.0], [16.0, 17.0]]);
        }
        Icon::Table => {
            polyline(&[
                [3.0, 5.0],
                [21.0, 5.0],
                [21.0, 19.0],
                [3.0, 19.0],
                [3.0, 5.0],
            ]);
            polyline(&[[3.0, 10.0], [21.0, 10.0]]);
            polyline(&[[3.0, 14.5], [21.0, 14.5]]);
            polyline(&[[9.0, 5.0], [9.0, 19.0]]);
        }
        Icon::Star => {
            let points: Vec<[f32; 2]> = (0u8..10)
                .map(|i| {
                    let angle =
                        std::f32::consts::PI * f32::from(i) / 5.0 - std::f32::consts::FRAC_PI_2;
                    let radius = if i % 2 == 0 { 9.5 } else { 4.0 };
                    [12.0 + radius * angle.cos(), 12.5 + radius * angle.sin()]
                })
                .collect();
            painter.add(egui::Shape::closed_line(
                points.into_iter().map(at).collect(),
                stroke,
            ));
        }
        Icon::Search => {
            painter.circle_stroke(at([10.0, 10.0]), 6.0 * scale, stroke);
            polyline(&[[14.5, 14.5], [20.0, 20.0]]);
        }
        Icon::NewFolder => {
            polyline(&[
                [2.0, 6.0],
                [8.0, 6.0],
                [10.0, 8.5],
                [19.0, 8.5],
                [19.0, 12.0],
            ]);
            polyline(&[[2.0, 6.0], [2.0, 19.0], [13.0, 19.0]]);
            polyline(&[[19.0, 15.0], [19.0, 21.0]]);
            polyline(&[[16.0, 18.0], [22.0, 18.0]]);
        }
        Icon::Preview => {
            polyline(&[
                [3.0, 5.0],
                [21.0, 5.0],
                [21.0, 19.0],
                [3.0, 19.0],
                [3.0, 5.0],
            ]);
            polyline(&[
                [3.0, 16.0],
                [9.0, 10.0],
                [13.0, 14.0],
                [16.0, 11.0],
                [21.0, 16.0],
            ]);
            painter.circle_stroke(at([16.5, 8.0]), 1.4 * scale, stroke);
        }
        Icon::Paste => {
            polyline(&[
                [5.0, 5.0],
                [19.0, 5.0],
                [19.0, 21.0],
                [5.0, 21.0],
                [5.0, 5.0],
            ]);
            polyline(&[[9.0, 5.0], [9.0, 3.0], [15.0, 3.0], [15.0, 5.0]]);
            polyline(&[[8.5, 12.0], [15.5, 12.0]]);
            polyline(&[[8.5, 16.0], [13.0, 16.0]]);
        }
        Icon::Home => {
            polyline(&[[3.0, 12.0], [12.0, 4.0], [21.0, 12.0]]);
            polyline(&[[6.0, 10.0], [6.0, 20.0], [18.0, 20.0], [18.0, 10.0]]);
            polyline(&[[10.0, 20.0], [10.0, 14.0], [14.0, 14.0], [14.0, 20.0]]);
        }
        Icon::Close => {
            polyline(&[[6.0, 6.0], [18.0, 18.0]]);
            polyline(&[[18.0, 6.0], [6.0, 18.0]]);
        }
        Icon::Check => polyline(&[[5.0, 12.5], [10.0, 17.5], [19.0, 7.0]]),
        Icon::Block => faces(BLOCK),
        Icon::Plate => {
            shade(
                &[[3.0, 12.0], [12.0, 7.5], [21.0, 12.0], [12.0, 16.5]],
                0.15,
            );
            shade(&[[3.0, 12.0], [12.0, 16.5], [12.0, 19.0], [3.0, 14.5]], 0.6);
            shade(
                &[[12.0, 16.5], [21.0, 12.0], [21.0, 14.5], [12.0, 19.0]],
                0.35,
            );
        }
        Icon::Beam => {
            // A horizontal member in isometric: lit top, mid front, dark end.
            shade(&[[3.0, 10.0], [15.0, 4.0], [21.0, 7.0], [9.0, 13.0]], 0.15);
            shade(&[[3.0, 10.0], [9.0, 13.0], [9.0, 17.0], [3.0, 14.0]], 0.6);
            shade(&[[9.0, 13.0], [21.0, 7.0], [21.0, 11.0], [9.0, 17.0]], 0.35);
        }
        Icon::Column => {
            // An upright member in isometric.
            shade(&[[12.0, 2.5], [16.5, 4.75], [12.0, 7.0], [7.5, 4.75]], 0.15);
            shade(&[[7.5, 4.75], [12.0, 7.0], [12.0, 21.5], [7.5, 19.25]], 0.6);
            shade(
                &[[12.0, 7.0], [16.5, 4.75], [16.5, 19.25], [12.0, 21.5]],
                0.35,
            );
        }
        Icon::Folder => faces(FOLDER),
        Icon::Recent => {
            painter.circle(at([12.0, 12.0]), 9.0 * scale, Color32::TRANSPARENT, stroke);
            polyline(&[[12.0, 7.0], [12.0, 12.0], [15.0, 14.0]]);
        }
        Icon::Shared => {
            painter.circle(at([12.0, 12.5]), 8.5 * scale, Color32::TRANSPARENT, stroke);
            polyline(&[[8.5, 13.0], [12.0, 9.5], [15.5, 13.0]]);
            polyline(&[[12.0, 9.5], [12.0, 16.5]]);
        }
        Icon::Plus => {
            polyline(&[[12.0, 5.0], [12.0, 19.0]]);
            polyline(&[[5.0, 12.0], [19.0, 12.0]]);
        }
        Icon::Back => polyline(&[[15.0, 6.0], [9.0, 12.0], [15.0, 18.0]]),
        Icon::Refresh => {
            polyline(&[
                [19.0, 11.0],
                [17.6, 7.0],
                [14.0, 4.6],
                [10.0, 4.6],
                [6.4, 7.0],
                [5.0, 11.0],
                [5.6, 15.0],
                [8.6, 18.4],
                [12.6, 19.4],
                [16.4, 18.0],
            ]);
            polyline(&[[19.0, 5.0], [19.0, 11.0], [13.0, 11.0]]);
        }
    }
}

/// Allocates a `size` x `size` icon in `ui`.
pub fn icon(ui: &mut Ui, icon: Icon, size: f32, line: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_icon(ui.painter(), rect, icon, line);
    response
}

/// Paints a generic template symbol, not a thumbnail of project geometry.
pub fn paint_preview(painter: &Painter, rect: Rect) {
    let palette = Palette::of(painter.ctx().theme());
    painter.rect_filled(rect, 4, palette.head);
    let block = Rect::from_center_size(rect.center(), Vec2::splat(rect.height() * 0.55));
    paint_icon(painter, block, Icon::Block, palette.line);
}

/// Where a PNG asset under `assets/icons/` may be: the working directory (an extracted
/// package), the source tree, next to the program, and in the macOS bundle's resources
/// (`bundled`, relative to `Contents/Resources`).
fn asset_candidates(file: &str, bundled: &str) -> Vec<PathBuf> {
    let name = Path::new("assets/icons").join(file);
    let mut candidates = vec![
        name.clone(),
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(&name),
    ];
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join(&name));
        candidates.push(dir.join("../Resources").join(bundled));
    }
    candidates
}

/// The official logo as `side` x `side` RGBA pixels, averaged down from the PNG; `None` when the
/// file is missing or does not decode.
pub fn logo_rgba(side: usize) -> Option<Vec<u8>> {
    let file = "beamcraft-1024.png";
    png_rgba(
        &asset_candidates(file, &format!("assets/icons/{file}")),
        side,
    )
}

/// The application icon (the official logo on a rounded tile; on Apple's grid on macOS,
/// edge to edge elsewhere) as `side` x `side` RGBA pixels; `None` when the file is missing or
/// does not decode.
pub fn app_icon_rgba(side: usize) -> Option<Vec<u8>> {
    let file = if cfg!(target_os = "macos") {
        "beamcraft-icon-macos.png"
    } else {
        "beamcraft-icon.png"
    };
    png_rgba(&asset_candidates(file, "beamcraft-icon.png"), side)
}

/// The first readable PNG of `candidates` as `side` x `side` RGBA pixels, averaged down.
fn png_rgba(candidates: &[PathBuf], side: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    let path = candidates.iter().find(|p| p.is_file())?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(8 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .ok()?;
    let icon = eframe::icon_data::from_png_bytes(&bytes).ok()?;
    let (w, h) = (
        usize::try_from(icon.width).ok()?,
        usize::try_from(icon.height).ok()?,
    );
    if w == 0 || h == 0 || side == 0 || icon.rgba.len() != w * h * 4 {
        return None;
    }
    // Box filter: every output pixel averages the source pixels it covers.
    let mut out = Vec::with_capacity(side * side * 4);
    for oy in 0..side {
        let (y0, y1) = (oy * h / side, ((oy + 1) * h / side).max(oy * h / side + 1));
        for ox in 0..side {
            let (x0, x1) = (ox * w / side, ((ox + 1) * w / side).max(ox * w / side + 1));
            let mut sum = [0u64; 4];
            let mut count = 0u64;
            for y in y0..y1.min(h) {
                for x in x0..x1.min(w) {
                    let i = (y * w + x) * 4;
                    for (c, s) in sum.iter_mut().enumerate() {
                        *s += u64::from(icon.rgba.get(i + c).copied().unwrap_or(0));
                    }
                    count += 1;
                }
            }
            for s in sum {
                out.push(u8::try_from(s / count.max(1)).unwrap_or(u8::MAX));
            }
        }
    }
    Some(out)
}

/// The official logo as a texture of `side` x `side` pixels ([`logo_rgba`]); `None` when it could
/// not be read (the window then shows the name only).
pub fn load_logo(ctx: &egui::Context, side: usize) -> Option<egui::TextureHandle> {
    let out = logo_rgba(side)?;
    let image = egui::ColorImage::from_rgba_unmultiplied([side, side], &out);
    Some(ctx.load_texture("beamcraft-logo", image, egui::TextureOptions::LINEAR))
}

/// Shows the unmodified logo at `size` points, or nothing when it
/// could not be loaded.
pub fn logo(ui: &mut Ui, texture: Option<&egui::TextureHandle>, size: f32) {
    let Some(texture) = texture else { return };
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let face = Rect::from_min_size(rect.min, Vec2::splat(size));
    let radius = CornerRadius::same(u8::try_from((size * 0.2) as i32).unwrap_or(10));
    let painter = ui.painter();
    painter.add(
        egui::epaint::RectShape::filled(face, radius, Color32::WHITE)
            .with_texture(texture.id(), Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0))),
    );
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

    use super::*;

    fn luminance(c: Color32) -> f64 {
        let lin = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.040_45 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn text_is_readable_in_both_themes() {
        for theme in [Theme::Light, Theme::Dark] {
            let p = Palette::of(theme);
            for ground in [p.page, p.surface, p.head, p.select] {
                assert!(contrast(p.text, ground) >= 7.0, "{theme:?} text");
                assert!(contrast(p.muted, ground) >= 4.5, "{theme:?} muted");
            }
            assert!(contrast(p.side_text(), p.side) >= 4.5, "{theme:?} side");
            assert!(contrast(p.link, p.surface) >= 4.5, "{theme:?} link");
            assert!(contrast(p.error, p.page) >= 4.5, "{theme:?} error");
        }
        assert!(contrast(INK, BRAND) >= 4.5, "text on the brand colour");
    }

    #[test]
    fn reinstalling_chrome_preserves_the_users_theme() {
        let ctx = egui::Context::default();
        install(&ctx);
        for preference in [
            egui::ThemePreference::Dark,
            egui::ThemePreference::Light,
            egui::ThemePreference::System,
        ] {
            ctx.set_theme(preference);
            install(&ctx);
            assert_eq!(ctx.options(|options| options.theme_preference), preference);
        }
        for theme in [Theme::Light, Theme::Dark] {
            let palette = Palette::of(theme);
            let panel = card(palette);
            assert_eq!(panel.stroke.width, 1.0);
            assert_eq!(panel.shadow, Shadow::NONE);
            assert_eq!(
                visuals(palette).widgets.inactive.corner_radius,
                CornerRadius::same(5)
            );
        }
    }

    #[test]
    fn system_preference_tracks_os_changes_but_explicit_preferences_do_not() {
        let ctx = egui::Context::default();
        install(&ctx);
        assert!(crate::app::inter(&ctx));
        for (preference, system, expected) in [
            (egui::ThemePreference::System, Theme::Light, Theme::Light),
            (egui::ThemePreference::System, Theme::Dark, Theme::Dark),
            (egui::ThemePreference::Light, Theme::Dark, Theme::Light),
            (egui::ThemePreference::Dark, Theme::Light, Theme::Dark),
        ] {
            ctx.set_theme(preference);
            let output = ctx.run_ui(
                egui::RawInput {
                    system_theme: Some(system),
                    ..Default::default()
                },
                |ui| {
                    ui.label("Inter");
                },
            );
            assert_eq!(ctx.theme(), expected);
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn the_official_logo_is_read_and_averaged_down() {
        // The source tree holds the logo, so a development build always finds it.
        let rgba = logo_rgba(256).unwrap();
        assert_eq!(rgba.len(), 256 * 256 * 4);
        // The logo fills its square: the centre is opaque.
        let centre = (128 * 256 + 128) * 4;
        assert_eq!(rgba[centre + 3], 255);
        assert!(logo_rgba(0).is_none());
    }

    #[test]
    fn the_application_icon_is_a_rounded_tile() {
        // The source tree holds the icon, so a development build always finds it.
        let rgba = app_icon_rgba(256).unwrap();
        assert_eq!(rgba.len(), 256 * 256 * 4);
        // macOS keeps the system-grid margin even when launched from the source tree.
        let alpha = |x: usize, y: usize| rgba[(y * 256 + x) * 4 + 3];
        assert_eq!(alpha(128, 128), 255);
        if cfg!(target_os = "macos") {
            assert_eq!(alpha(128, 0), 0);
            assert_eq!(alpha(128, 24), 0);
            assert_eq!(alpha(128, 26), 255);
            assert_eq!(alpha(128, 229), 255);
            assert_eq!(alpha(128, 231), 0);
        } else {
            assert_eq!(alpha(128, 0), 255);
        }
        assert_eq!(alpha(1, 1), 0);
        assert!(app_icon_rgba(0).is_none());
    }

    #[test]
    fn icon_faces_are_convex() {
        for polygons in [BLOCK, FOLDER, DELETE] {
            for (points, _) in polygons {
                let n = points.len();
                let mut sign = 0.0f32;
                for i in 0..n {
                    let [a, b, c] = [points[i], points[(i + 1) % n], points[(i + 2) % n]];
                    let cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
                    if cross.abs() > 1e-6 {
                        assert!(sign == 0.0 || sign.signum() == cross.signum(), "{points:?}");
                        sign = cross;
                    }
                }
            }
        }
    }
}
