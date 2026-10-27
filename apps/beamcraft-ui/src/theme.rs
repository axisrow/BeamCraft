// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The BeamCraft look, taken from the logo: brand cyan with a lighter blueprint grid, cream faces,
//! grey-teal sides and a heavy ink outline. Widgets get a 2 px ink border, 10 px corners and a hard
//! offset shadow; the light and the dark theme follow the operating system.
//!
//! Icons are contributor-original isometric glyphs in the same manner (cream top, teal sides, ink
//! outline), painted from convex polygons so no icon file is shipped. The logo itself is shown
//! unmodified from the official `assets/icons/beamcraft-1024.png`, read at run time like the font.

use egui::{
    Color32, CornerRadius, FontFamily, FontId, Painter, Pos2, Rect, Response, RichText, Sense,
    Shadow, Stroke, StrokeKind, Theme, Ui, Vec2, pos2, vec2,
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
    /// Borders and hard shadows.
    pub ink: Color32,
    /// Outline of icons drawn on the page.
    pub line: Color32,
    /// Body text.
    pub text: Color32,
    /// Secondary text (at least 4.5:1 on `page` and `surface`).
    pub muted: Color32,
    /// Thin separators.
    pub rule: Color32,
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
                page: CREAM,
                surface: Color32::from_rgb(0xFF, 0xFD, 0xF6),
                head: Color32::from_rgb(0xED, 0xE5, 0xCF),
                ink: INK,
                line: INK,
                text: INK,
                muted: Color32::from_rgb(0x4D, 0x5A, 0x60),
                rule: Color32::from_rgb(0xE0, 0xD7, 0xC0),
                side: BRAND,
                side_grid: BRAND_LIGHT,
                select: Color32::from_rgb(0xD3, 0xF1, 0xF8),
                link: Color32::from_rgb(0x00, 0x6F, 0x87),
                error: Color32::from_rgb(0xA8, 0x1E, 0x12),
            },
            Theme::Dark => Self {
                dark: true,
                page: Color32::from_rgb(0x0E, 0x15, 0x17),
                surface: Color32::from_rgb(0x17, 0x22, 0x26),
                head: Color32::from_rgb(0x1D, 0x2B, 0x30),
                ink: Color32::BLACK,
                line: CREAM,
                text: CREAM,
                muted: Color32::from_rgb(0xA9, 0xB8, 0xBD),
                rule: Color32::from_rgb(0x2A, 0x3A, 0x40),
                side: Color32::from_rgb(0x0B, 0x34, 0x40),
                side_grid: Color32::from_rgb(0x12, 0x4B, 0x59),
                select: Color32::from_rgb(0x0E, 0x3B, 0x46),
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
        if self.dark { CREAM } else { INK }
    }
}

fn visuals(palette: Palette) -> egui::Visuals {
    let mut v = if palette.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let radius = CornerRadius::same(10);
    let border = Stroke::new(2.0, palette.ink);
    v.panel_fill = palette.page;
    v.window_fill = palette.surface;
    v.window_stroke = border;
    v.window_corner_radius = CornerRadius::same(14);
    v.window_shadow = Shadow {
        offset: [6, 6],
        blur: 0,
        spread: 0,
        color: palette.ink,
    };
    v.popup_shadow = Shadow {
        offset: [3, 3],
        blur: 0,
        spread: 0,
        color: palette.ink,
    };
    v.menu_corner_radius = radius;
    v.extreme_bg_color = palette.surface;
    v.faint_bg_color = palette.head;
    v.hyperlink_color = palette.link;
    v.error_fg_color = palette.error;
    v.override_text_color = Some(palette.text);
    v.selection.bg_fill = BRAND;
    v.selection.stroke = Stroke::new(1.5, INK);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = palette.surface;
    w.noninteractive.weak_bg_fill = palette.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.5, palette.rule);
    w.noninteractive.fg_stroke = Stroke::new(1.0, palette.text);
    w.noninteractive.corner_radius = radius;
    for (state, fill) in [
        (&mut w.inactive, palette.surface),
        (&mut w.hovered, palette.head),
        (&mut w.active, BRAND),
        (&mut w.open, palette.head),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = border;
        state.corner_radius = radius;
        state.expansion = 0.0;
    }
    w.inactive.fg_stroke = Stroke::new(1.5, palette.text);
    w.hovered.fg_stroke = Stroke::new(1.5, palette.text);
    w.open.fg_stroke = Stroke::new(1.5, palette.text);
    w.active.fg_stroke = Stroke::new(1.5, INK);
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
    ctx.set_theme(preference);
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = vec2(10.0, 10.0);
        style.spacing.button_padding = vec2(14.0, 8.0);
        style.spacing.interact_size.y = 36.0;
        style.text_styles.insert(
            egui::TextStyle::Body,
            FontId::new(14.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            FontId::new(14.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Small,
            FontId::new(12.0, FontFamily::Proportional),
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

/// A frame with an ink border and a hard offset shadow of `offset` points.
pub fn card(palette: Palette, offset: i8) -> egui::Frame {
    egui::Frame::new()
        .fill(palette.surface)
        .stroke(Stroke::new(2.0, palette.ink))
        .corner_radius(CornerRadius::same(12))
        .shadow(Shadow {
            offset: [offset, offset],
            blur: 0,
            spread: 0,
            color: palette.ink,
        })
}

/// How a painted button looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Brand cyan with ink text: the one main action of a view.
    Primary,
    /// Surface fill with an ink border.
    Secondary,
}

/// A button in the BeamCraft style: ink border, hard shadow that the button sinks into when
/// pressed. Disabled buttons lose the shadow and dim.
pub fn button(ui: &mut Ui, kind: Kind, text: &str, enabled: bool) -> Response {
    let palette = Palette::current(ui);
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        FontId::new(14.0, bold_family()),
        Color32::PLACEHOLDER,
    );
    let size = vec2((galley.size().x + 36.0).max(64.0), 42.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size + vec2(3.0, 3.0), sense);
    let face = Rect::from_min_size(rect.min, size);
    let pressed = enabled && response.is_pointer_button_down_on();
    let face = if pressed {
        face.translate(vec2(2.0, 2.0))
    } else {
        face
    };
    let (fill, ink) = match kind {
        Kind::Primary => (BRAND, INK),
        Kind::Secondary if response.hovered() && enabled => (palette.head, palette.text),
        Kind::Secondary => (palette.surface, palette.text),
    };
    let painter = ui.painter();
    let radius = CornerRadius::same(10);
    if enabled && !pressed {
        painter.rect_filled(face.translate(vec2(3.0, 3.0)), radius, palette.ink);
    }
    let alpha = if enabled { 1.0 } else { 0.45 };
    painter.rect_filled(face, radius, fill.gamma_multiply(alpha));
    painter.rect_stroke(
        face,
        radius,
        Stroke::new(2.0, palette.ink.gamma_multiply(alpha)),
        StrokeKind::Inside,
    );
    painter.galley_with_override_text_color(
        face.center() - galley.size() / 2.0,
        galley,
        ink.gamma_multiply(alpha),
    );
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
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
}

#[derive(Clone, Copy)]
enum Face {
    Cream,
    Teal,
    TealDark,
    Brand,
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
const PLATE: &[Polygon] = &[
    (
        &[[3.0, 12.0], [3.0, 14.5], [12.0, 19.0], [12.0, 16.5]],
        Face::TealDark,
    ),
    (
        &[[12.0, 16.5], [21.0, 12.0], [21.0, 14.5], [12.0, 19.0]],
        Face::Teal,
    ),
    (
        &[[3.0, 12.0], [12.0, 7.5], [21.0, 12.0], [12.0, 16.5]],
        Face::Cream,
    ),
];
const BEAM: &[Polygon] = &[
    (
        &[[7.0, 11.0], [17.0, 7.0], [17.0, 11.0], [7.0, 15.0]],
        Face::TealDark,
    ),
    (
        &[[9.0, 15.0], [19.0, 11.0], [19.0, 13.0], [9.0, 17.0]],
        Face::Teal,
    ),
    (
        &[[3.0, 9.0], [13.0, 5.0], [19.0, 5.0], [9.0, 9.0]],
        Face::Cream,
    ),
    (
        &[[9.0, 9.0], [19.0, 5.0], [19.0, 7.0], [9.0, 11.0]],
        Face::Teal,
    ),
    (
        &[[3.0, 9.0], [9.0, 9.0], [9.0, 11.0], [3.0, 11.0]],
        Face::Cream,
    ),
    (
        &[[5.0, 11.0], [7.0, 11.0], [7.0, 15.0], [5.0, 15.0]],
        Face::Cream,
    ),
    (
        &[[3.0, 15.0], [9.0, 15.0], [9.0, 17.0], [3.0, 17.0]],
        Face::Cream,
    ),
];
const COLUMN: &[Polygon] = &[
    (
        &[[10.5, 6.0], [13.5, 6.0], [13.5, 20.0], [10.5, 20.0]],
        Face::TealDark,
    ),
    (
        &[[6.0, 4.0], [10.5, 4.0], [10.5, 21.0], [6.0, 21.0]],
        Face::Cream,
    ),
    (
        &[[13.5, 4.0], [18.0, 4.0], [18.0, 21.0], [13.5, 21.0]],
        Face::Teal,
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

const BOLT: &[Polygon] = &[
    (
        &[[11.0, 10.5], [13.0, 10.5], [13.0, 21.0], [11.0, 21.0]],
        Face::Teal,
    ),
    (
        &[[7.0, 6.0], [7.0, 8.5], [12.0, 11.0], [12.0, 8.5]],
        Face::TealDark,
    ),
    (
        &[[12.0, 8.5], [12.0, 11.0], [17.0, 8.5], [17.0, 6.0]],
        Face::Teal,
    ),
    (
        &[[7.0, 6.0], [12.0, 3.5], [17.0, 6.0], [12.0, 8.5]],
        Face::Cream,
    ),
];
const WELD: &[Polygon] = &[
    (
        &[[3.0, 16.0], [12.0, 11.5], [21.0, 16.0], [12.0, 20.5]],
        Face::Cream,
    ),
    (
        &[[8.0, 13.5], [8.0, 5.0], [16.0, 9.0], [16.0, 17.5]],
        Face::Teal,
    ),
    (
        &[[8.0, 13.5], [16.0, 17.5], [14.5, 18.5], [6.5, 14.5]],
        Face::Brand,
    ),
];
const COPY: &[Polygon] = &[
    (
        &[[8.0, 3.0], [20.0, 3.0], [20.0, 15.0], [8.0, 15.0]],
        Face::Teal,
    ),
    (
        &[[4.0, 9.0], [16.0, 9.0], [16.0, 21.0], [4.0, 21.0]],
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
        for (points, face) in polygons {
            let fill = match face {
                Face::Cream => CREAM,
                Face::Teal => TEAL,
                Face::TealDark => TEAL_DARK,
                Face::Brand => BRAND,
            };
            painter.add(egui::Shape::convex_polygon(
                points.iter().map(|p| at(*p)).collect(),
                fill,
                stroke,
            ));
        }
    };
    let polyline = |points: &[[f32; 2]]| {
        painter.add(egui::Shape::line(
            points.iter().map(|p| at(*p)).collect(),
            stroke,
        ));
    };
    match icon {
        Icon::Select => {
            // Star-shaped from its tip: a fan of two triangles, then the outline.
            let [a, b, c, d] = [[5.0, 3.0], [18.0, 11.0], [12.0, 12.5], [9.0, 19.0]];
            for triangle in [[a, b, c], [a, c, d]] {
                painter.add(egui::Shape::convex_polygon(
                    triangle.map(at).to_vec(),
                    CREAM,
                    Stroke::NONE,
                ));
            }
            let tip = [a, b, c, d];
            painter.add(egui::Shape::closed_line(tip.map(at).to_vec(), stroke));
        }
        Icon::Bolt => faces(BOLT),
        Icon::Weld => faces(WELD),
        Icon::Copy => faces(COPY),
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
        Icon::Block => faces(BLOCK),
        Icon::Plate => faces(PLATE),
        Icon::Beam => faces(BEAM),
        Icon::Column => faces(COLUMN),
        Icon::Folder => faces(FOLDER),
        Icon::Recent => {
            painter.circle(at([12.0, 12.0]), 9.0 * scale, CREAM, stroke);
            polyline(&[[12.0, 7.0], [12.0, 12.0], [15.0, 14.0]]);
        }
        Icon::Shared => {
            painter.circle(at([12.0, 12.5]), 8.5 * scale, CREAM, stroke);
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

/// Paints the preview tile of a model: brand cyan, blueprint grid, an isometric block.
pub fn paint_preview(painter: &Painter, rect: Rect) {
    let radius = CornerRadius::same(10);
    painter.rect_filled(rect, radius, BRAND);
    let inner = rect.shrink(2.0);
    let clip = painter.with_clip_rect(inner);
    let step = rect.height() / 5.0;
    let mut x = rect.left() + step;
    while x < rect.right() {
        clip.line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(1.2, BRAND_LIGHT),
        );
        x += step;
    }
    let mut y = rect.top() + step;
    while y < rect.bottom() {
        clip.line_segment(
            [pos2(rect.left(), y), pos2(rect.right(), y)],
            Stroke::new(1.2, BRAND_LIGHT),
        );
        y += step;
    }
    let block = Rect::from_center_size(rect.center(), Vec2::splat(rect.height() * 0.85));
    paint_icon(painter, block, Icon::Plate, INK);
    painter.rect_stroke(rect, radius, Stroke::new(2.0, INK), StrokeKind::Inside);
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

/// Shows the logo at `size` points with an ink border and a hard shadow, or nothing when it
/// could not be loaded.
pub fn logo(ui: &mut Ui, texture: Option<&egui::TextureHandle>, size: f32) {
    let Some(texture) = texture else { return };
    let palette = Palette::current(ui);
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size + 3.0), Sense::hover());
    let face = Rect::from_min_size(rect.min, Vec2::splat(size));
    let radius = CornerRadius::same(u8::try_from((size * 0.2) as i32).unwrap_or(10));
    let painter = ui.painter();
    painter.rect_filled(face.translate(vec2(3.0, 3.0)), radius, palette.ink);
    painter.add(
        egui::epaint::RectShape::filled(face, radius, Color32::WHITE)
            .with_texture(texture.id(), Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0))),
    );
    painter.rect_stroke(
        face,
        radius,
        Stroke::new(2.0, palette.ink),
        StrokeKind::Inside,
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
        for polygons in [BLOCK, PLATE, BEAM, COLUMN, FOLDER, BOLT, WELD, COPY, DELETE] {
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
