// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The shared widgets of the side panels: one disclosure chevron, the collapsible group bar of
//! a properties grid, the lighter fold of a sub-block, the two-column property row and the
//! spacing tokens they share. Every panel draws its groups and rows through these, so the
//! project browser, the properties of a view, a part, a bolt or a grid all look alike.

use crate::theme::Palette;
use egui::collapsing_header::CollapsingState;
use egui::{Align, Align2, FontId, Rect, Response, Sense, Stroke, Ui, pos2, vec2};

/// The smallest gap: between an icon and its text, under a group bar.
pub const SPACE_XS: f32 = 4.0;
/// The gap between the rows of a block and the inner margin of a panel.
pub const SPACE_S: f32 = 8.0;
/// The gap between blocks.
pub const SPACE_M: f32 = 12.0;
/// The gap between sections of a panel.
pub const SPACE_L: f32 = 16.0;
/// The height of one row of a properties grid, a group bar or a tree row.
pub const ROW_HEIGHT: f32 = 24.0;
/// The width of the label column of a properties grid. Wide enough for the longest row label
/// with its unit ("On plane offset, mm"), so the ellipsis never hides a unit.
pub const LABEL_WIDTH: f32 = 136.0;
/// The square a disclosure chevron is drawn in.
pub const CHEVRON: f32 = 16.0;
/// Slack left at the end of a row that fills the panel: a row sized to the exact available
/// width can come out a pixel wider, and a resizable panel would follow it wider every frame.
pub const FIT: f32 = 6.0;

/// Paints the disclosure chevron centred on `center`: pointing right when closed (`openness`
/// 0), turning down as the block opens (1). Two strokes of one weight, the way the icon set is
/// drawn, rather than a filled triangle.
pub fn chevron(painter: &egui::Painter, center: egui::Pos2, openness: f32, color: egui::Color32) {
    let angle = openness.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2;
    let (sin, cos) = angle.sin_cos();
    // The right-pointing chevron, then rotated a quarter turn clockwise as it opens.
    let turn = |x: f32, y: f32| center + vec2(x * cos - y * sin, x * sin + y * cos);
    painter.line(
        vec![turn(-1.75, -3.5), turn(1.75, 0.0), turn(-1.75, 3.5)],
        Stroke::new(1.5, color),
    );
}

/// The chevron as an egui toggle-button painter, for a [`CollapsingState`] header.
pub fn paint_chevron(ui: &mut Ui, openness: f32, response: &Response) {
    let palette = Palette::current(ui);
    let color = if response.hovered() {
        palette.text
    } else {
        palette.muted
    };
    chevron(ui.painter(), response.rect.center(), openness, color);
}

/// Draws one clickable header row of `width` with a chevron at its left and `title` beside it,
/// toggling `state` when clicked. `fill` paints the row first (the group bar's band).
fn header_row(
    ui: &mut Ui,
    state: &mut CollapsingState,
    title: &str,
    strong: bool,
    fill: Option<egui::Color32>,
) -> Response {
    let palette = Palette::current(ui);
    let width = (ui.available_width() - FIT).max(40.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click());
    if response.clicked() {
        state.toggle(ui);
    }
    let openness = state.openness(ui.ctx());
    let painter = ui.painter();
    let hovered = response.hovered() || response.has_focus();
    match (fill, hovered) {
        (Some(fill), false) => {
            painter.rect_filled(rect, 3, fill);
        }
        (_, true) => {
            painter.rect_filled(rect, 3, palette.select);
        }
        (None, false) => {}
    }
    chevron(
        painter,
        pos2(rect.left() + SPACE_S + CHEVRON / 2.0 - 4.0, rect.center().y),
        openness,
        if hovered { palette.text } else { palette.muted },
    );
    let font = FontId::proportional(12.0);
    let galley = painter.layout_no_wrap(title.to_owned(), font, palette.text);
    let text_at = pos2(
        rect.left() + SPACE_S + CHEVRON,
        rect.center().y - galley.size().y / 2.0,
    );
    let clip = Rect::from_min_max(pos2(text_at.x, rect.top()), rect.max);
    if strong {
        // A faux bold: the same text a hair to the right, so the bar title reads as a heading
        // without loading a second font.
        painter
            .with_clip_rect(clip)
            .galley(text_at + vec2(0.4, 0.0), galley.clone(), palette.text);
    }
    painter
        .with_clip_rect(clip)
        .galley(text_at, galley, palette.text);
    let open = state.is_open();
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::DisclosureTriangle, true, open, title)
    });
    response
}

/// A collapsible group of a properties grid: a filled bar with a chevron and the group's
/// title, and its rows under it. `title` also keys the open state, so it must be unique in its
/// panel; `salt` tells two panels' groups of one title apart.
pub fn section<R>(
    ui: &mut Ui,
    salt: &str,
    title: &str,
    default_open: bool,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let palette = Palette::current(ui);
    let id = ui.make_persistent_id(("kit-section", salt, title));
    let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, default_open);
    header_row(ui, &mut state, title, true, Some(palette.head));
    // The rows sit tight under the bar as one table: no gap between the cells.
    let out = state.show_body_unindented(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        add(ui)
    });
    ui.add_space(SPACE_XS);
    out.map(|r| r.inner)
}

/// A lighter disclosure inside a block (precise placement, line limits, the work plane): no
/// band, the chevron and the title only, and its content indented under the title.
pub fn fold<R>(
    ui: &mut Ui,
    salt: &str,
    title: &str,
    default_open: bool,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let id = ui.make_persistent_id(("kit-fold", salt, title));
    let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, default_open);
    header_row(ui, &mut state, title, false, None);
    let out = state.show_body_unindented(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(SPACE_S + CHEVRON - ui.spacing().item_spacing.x);
            ui.vertical(add).inner
        })
        .inner
    });
    out.map(|r| r.inner)
}

/// A tree node: a chevron toggle and a header drawn beside it on one row
/// ([`TreeNode::header`]), then its body indented under it while open ([`TreeNode::body`]).
/// The two halves are separate calls, so the header and the body may borrow the same state.
pub struct TreeNode {
    state: CollapsingState,
}

impl TreeNode {
    /// Draws the header row of the node keyed by `id`: the chevron, then `header`. Returns
    /// the node, the row's response and what the header returned.
    pub fn header<H>(
        ui: &mut Ui,
        id: egui::Id,
        default_open: bool,
        header: impl FnOnce(&mut Ui) -> H,
    ) -> (Self, Response, H) {
        let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, default_open);
        let row = ui.horizontal(|ui| {
            ui.set_min_height(ROW_HEIGHT);
            ui.spacing_mut().item_spacing.x = SPACE_XS;
            let (_, toggle) = ui.allocate_exact_size(vec2(CHEVRON, ROW_HEIGHT), Sense::click());
            if toggle.clicked() {
                state.toggle(ui);
            }
            let openness = state.openness(ui.ctx());
            paint_chevron(ui, openness, &toggle);
            header(ui)
        });
        (Self { state }, row.response, row.inner)
    }

    /// Draws the body under the header while the node is open, indented to the header's icon
    /// with a guide line down from the chevron, and stores the node's state.
    pub fn body<B>(mut self, ui: &mut Ui, body: impl FnOnce(&mut Ui) -> B) -> Option<B> {
        let opened = self.state.show_body_unindented(ui, |ui| {
            ui.horizontal(|ui| {
                let palette = Palette::current(ui);
                let guide_x = ui.cursor().left() + CHEVRON / 2.0;
                ui.add_space(CHEVRON);
                let inner = ui.vertical(body);
                let r = inner.response.rect;
                ui.painter().vline(
                    guide_x,
                    r.top()..=r.bottom(),
                    Stroke::new(1.0, palette.rule.gamma_multiply(0.7)),
                );
                inner.inner
            })
            .inner
        });
        opened.map(|r| r.inner)
    }
}

/// Toggles the tree node keyed by `id`, as a click on its name does.
pub fn toggle_node(ui: &Ui, id: egui::Id, default_open: bool) {
    let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, default_open);
    state.toggle(ui);
    state.store(ui.ctx());
}

/// The type box at the top of a properties panel, as Revit's properties palette shows the type
/// of what is selected: a framed icon and the kind, nothing the rows below repeat.
pub fn type_box(ui: &mut Ui, icon: crate::theme::Icon, kind: &str) {
    let palette = Palette::current(ui);
    egui::Frame::new()
        .stroke(Stroke::new(1.0, palette.rule))
        .fill(palette.surface)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .corner_radius(4)
        .show(ui, |ui| {
            ui.set_width((ui.available_width() - FIT).max(40.0));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE_S;
                crate::theme::icon(ui, icon, 22.0, palette.text);
                ui.add(egui::Label::new(egui::RichText::new(kind).strong().size(13.0)).truncate());
            });
        });
    ui.add_space(SPACE_S);
}

/// Records the window area below the model header and ribbon, where floating windows open
/// and stay. Set every frame by the model window once its chrome is laid out.
pub fn set_work_area(ctx: &egui::Context, rect: Rect) {
    ctx.data_mut(|d| d.insert_temp(work_area_id(), rect));
}

/// The area floating windows open in and are kept within: below the header and ribbon when
/// the model window recorded them, else the whole content area (the start page, tests).
fn work_area(ctx: &egui::Context) -> Rect {
    let content = ctx.content_rect();
    ctx.data(|d| d.get_temp::<Rect>(work_area_id()))
        .map(|area| content.intersect(area))
        .filter(|area| area.is_positive())
        .unwrap_or(content)
}

fn work_area_id() -> egui::Id {
    egui::Id::unique("floating-work-area")
}

/// A settings window of the properties panel (Visibility/Graphics, Graphic Display, Color
/// Scheme), as a CAD program opens its dialogs: a plain title, centred below the ribbon the
/// first time and kept below it, resizable, closed by its cross. A tall dialog centred on the
/// whole screen rose over the ribbon buttons (the recheck of issue 562 item 46).
pub fn dialog<'a>(
    title: &str,
    id: &str,
    open: &'a mut bool,
    ctx: &egui::Context,
) -> egui::Window<'a> {
    egui::Window::new(egui::RichText::new(title).size(13.0).strong())
        .id(egui::Id::unique(id))
        .open(open)
        .collapsible(false)
        .resizable(true)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(work_area(ctx).center())
        .constrain_to(work_area(ctx))
        .default_width(380.0)
}

/// A floating tool window (Details, the sheet editor, the drawing document): it opens centred
/// on the content area, never over the header and ribbon that a default position would cover.
pub fn tool_window<'a>(title: &str, open: &'a mut bool, ctx: &egui::Context) -> egui::Window<'a> {
    egui::Window::new(egui::RichText::new(title).size(13.0).strong())
        .open(open)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(work_area(ctx).center())
        .constrain_to(work_area(ctx))
}

/// A search field with its magnifier inside the frame, filling the row up to `reserve`
/// points kept free at its end. Returns the text edit's response.
pub fn search_field(ui: &mut Ui, text: &mut String, hint: &str, reserve: f32) -> Response {
    let palette = Palette::current(ui);
    let width = (ui.available_width() - reserve - FIT).max(60.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT + 2.0), Sense::hover());
    let edit = ui.put(
        rect.shrink2(vec2(0.0, 1.0)),
        egui::TextEdit::singleline(text)
            .hint_text(hint)
            .char_limit(128)
            .margin(egui::Margin {
                left: 26,
                right: 6,
                top: 4,
                bottom: 4,
            }),
    );
    crate::theme::paint_icon(
        ui.painter(),
        Rect::from_center_size(pos2(rect.left() + 13.0, rect.center().y), vec2(14.0, 14.0)),
        crate::theme::Icon::Search,
        palette.muted,
    );
    edit
}

/// Marks the next table row as changed from its saved value: the row shows a dot in its label
/// cell until Apply or Reset. The mark is consumed by that row.
pub fn mark_next_changed(ui: &Ui, changed: bool) {
    if changed {
        ui.data_mut(|d| d.insert_temp(changed_id(), true));
    }
}

fn changed_id() -> egui::Id {
    egui::Id::unique("kit-next-row-changed")
}

/// One row of a properties table, as Revit's properties palette lays them out: the label
/// cell on the left, a rule between the cells and the value cell on the right, a rule under
/// the row. The value's widgets are flat until they are hovered or edited, so the table reads
/// as text.
pub fn prop_row<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let palette = Palette::current(ui);
    let width = (ui.available_width() - FIT).max(60.0);
    let row = ui.horizontal(|ui| {
        ui.set_min_height(ROW_HEIGHT);
        ui.spacing_mut().item_spacing.x = 0.0;
        let (label_rect, _) = ui.allocate_exact_size(vec2(LABEL_WIDTH, ROW_HEIGHT), Sense::hover());
        let changed = ui
            .data_mut(|d| d.remove_temp::<bool>(changed_id()))
            .unwrap_or(false);
        let painter = ui.painter();
        if changed {
            painter.circle_filled(
                pos2(label_rect.left() + 3.0, label_rect.center().y),
                2.5,
                palette.link,
            );
        }
        let galley = painter.layout(
            label.to_owned(),
            FontId::proportional(12.0),
            palette.text,
            LABEL_WIDTH - SPACE_S * 1.5,
        );
        let shown = if galley.rows.len() > 1 {
            // A label longer than its cell is cut with an ellipsis, as the palette does.
            let mut job = egui::text::LayoutJob::simple_singleline(
                label.to_owned(),
                FontId::proportional(12.0),
                palette.text,
            );
            job.wrap = egui::text::TextWrapping::truncate_at_width(LABEL_WIDTH - SPACE_S * 1.5);
            painter.layout_job(job)
        } else {
            galley
        };
        painter.galley(
            pos2(
                label_rect.left() + SPACE_S,
                label_rect.center().y - shown.size().y / 2.0,
            ),
            shown,
            palette.text,
        );
        ui.add_space(SPACE_XS);
        ui.scope(|ui| {
            // Flat value cells: no frame or fill until the pointer is on the widget.
            let w = &mut ui.visuals_mut().widgets;
            w.inactive.bg_stroke = Stroke::NONE;
            w.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
            w.inactive.bg_fill = egui::Color32::TRANSPARENT;
            ui.visuals_mut().extreme_bg_color = egui::Color32::TRANSPARENT;
            ui.set_width((width - LABEL_WIDTH - SPACE_XS).max(30.0));
            add(ui)
        })
        .inner
    });
    let rect = row.response.rect;
    let line = Stroke::new(1.0, palette.rule.gamma_multiply(0.7));
    let painter = ui.painter();
    painter.hline(rect.left()..=rect.left() + width, rect.max.y, line);
    painter.vline(rect.left() + LABEL_WIDTH, rect.y_range(), line);
    if row.response.hovered() {
        painter.rect_filled(
            Rect::from_min_size(rect.min, vec2(LABEL_WIDTH, rect.height())),
            0,
            palette.head.gamma_multiply(0.5),
        );
    }
    row.inner
}

/// A table row whose value is a button filling the cell: the row of a setting edited in its
/// own window (Visibility/Graphics, Graphic Display, Color Scheme). Returns whether it was
/// clicked.
pub fn prop_button(ui: &mut Ui, label: &str, text: &str) -> bool {
    prop_row(ui, label, |ui| {
        let palette = Palette::current(ui);
        let width = (ui.available_width() - SPACE_XS).max(30.0);
        // One line, cut with an ellipsis and whole on hover: a long summary such as
        // "Edit… (0 filters, 1 hidden)" wrapped onto a second line past the row height.
        ui.add_sized(
            [width, ROW_HEIGHT - 4.0],
            egui::Button::new(egui::RichText::new(text).size(12.0))
                .wrap_mode(egui::TextWrapMode::Truncate)
                .fill(palette.head)
                .stroke(Stroke::new(1.0, palette.rule)),
        )
        .on_hover_text(text)
        .clicked()
    })
}

/// A text property in a table row: the field fills the value cell.
pub fn prop_text(ui: &mut Ui, label: &str, text: &mut String) -> Response {
    prop_row(ui, label, |ui| {
        ui.add(
            egui::TextEdit::singleline(text)
                .desired_width((ui.available_width() - SPACE_XS).max(30.0))
                .char_limit(4096),
        )
    })
}

/// A yes/no property in a table row.
pub fn prop_check(ui: &mut Ui, label: &str, value: &mut bool) -> Response {
    prop_row(ui, label, |ui| ui.checkbox(value, ""))
}

/// A property chosen from a short fixed list, in a table row.
pub fn prop_choice<T: PartialEq + Copy>(
    ui: &mut Ui,
    id: &str,
    label: &str,
    value: &mut T,
    choices: &[(T, &str)],
) {
    prop_row(ui, label, |ui| {
        let shown = choices
            .iter()
            .find(|(c, _)| c == value)
            .map_or("", |(_, name)| name);
        egui::ComboBox::from_id_salt(id)
            .width((ui.available_width() - SPACE_XS).max(30.0))
            .selected_text(shown)
            .truncate()
            .show_ui(ui, |ui| {
                for (choice, name) in choices {
                    ui.selectable_value(value, *choice, *name);
                }
            });
    });
}

/// A picked value shown read-only in a table row, or `missing` muted while nothing is picked.
pub fn prop_info(ui: &mut Ui, label: &str, value: Option<&str>, missing: &str) {
    prop_row(ui, label, |ui| {
        let palette = Palette::current(ui);
        match value {
            Some(text) => ui.add(egui::Label::new(egui::RichText::new(text).size(12.0)).truncate()),
            None => ui.label(egui::RichText::new(missing).size(12.0).color(palette.muted)),
        }
    });
}

/// What the action bar of an editor was asked to do this frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Actions {
    /// The main action (Apply, Create, Update) was pressed.
    pub primary: bool,
    /// Cancel or Close was pressed.
    pub cancel: bool,
}

/// The action bar every editor of the properties panel ends with: the main action filled, then
/// Cancel, at the full width of the panel and the same height everywhere; a spinner while a
/// command runs.
pub fn actions(
    ui: &mut Ui,
    primary: &str,
    primary_enabled: bool,
    cancel: &str,
    busy: bool,
) -> Actions {
    let mut out = Actions::default();
    ui.add_space(SPACE_S);
    ui.separator();
    ui.add_space(SPACE_XS);
    ui.columns(2, |columns| {
        if let [left, right] = columns {
            out.primary = left
                .with_layout(
                    egui::Layout::top_down_justified(egui::Align::Center),
                    |ui| {
                        crate::theme::button(
                            ui,
                            crate::theme::Kind::Primary,
                            primary,
                            primary_enabled && !busy,
                        )
                        .clicked()
                    },
                )
                .inner;
            out.cancel = right
                .with_layout(
                    egui::Layout::top_down_justified(egui::Align::Center),
                    |ui| {
                        crate::theme::button(ui, crate::theme::Kind::Secondary, cancel, !busy)
                            .clicked()
                    },
                )
                .inner;
        }
    });
    if busy {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(egui::RichText::new("Running…").size(11.0));
        });
    }
    out
}

/// A read-only property row: the value right-aligned in the value column, with its unit muted
/// after it, so numbers line up down the grid. A value wider than the cell wraps onto a second
/// line instead of running left over the label column.
pub fn prop_value(ui: &mut Ui, label: &str, value: &str, unit: Option<&str>) {
    prop_row(ui, label, |ui| {
        let palette = Palette::current(ui);
        let width = (ui.available_width() - FIT).max(40.0);
        let unit_width = unit.map_or(0.0, |unit| {
            ui.painter()
                .layout_no_wrap(unit.to_owned(), FontId::proportional(11.0), palette.muted)
                .size()
                .x
                + SPACE_XS * 2.0
        });
        // The value keeps its right edge where a one-line value ends, so single-line rows
        // (the common case) paint exactly as before; only a too-wide value wraps.
        let value_width = (width - unit_width).max(40.0);
        let mut job = egui::text::LayoutJob::simple(
            value.to_owned(),
            FontId::proportional(12.0),
            palette.text,
            value_width,
        );
        job.halign = Align::RIGHT;
        let galley = ui.painter().layout_job(job);
        let (rect, response) = ui.allocate_exact_size(
            vec2(width, ROW_HEIGHT - 4.0).max(vec2(width, galley.size().y)),
            Sense::hover(),
        );
        let painter = ui.painter_at(rect);
        // A right-aligned galley lays its rows out to the left of the anchor, so anchoring at
        // the value's right edge keeps single-line rows exactly where they were.
        painter.galley(
            pos2(
                rect.right() - unit_width,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            palette.text,
        );
        if let Some(unit) = unit {
            let galley =
                painter.layout_no_wrap(unit.to_owned(), FontId::proportional(11.0), palette.muted);
            painter.galley(
                pos2(
                    rect.right() - galley.size().x - SPACE_XS,
                    rect.center().y - galley.size().y / 2.0,
                ),
                galley,
                palette.muted,
            );
        }
        response.on_hover_text(match unit {
            Some(unit) => format!("{value} {unit}"),
            None => value.to_owned(),
        });
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn frame(ctx: &egui::Context, events: Vec<egui::Event>, run: impl FnMut(&mut Ui)) {
        let input = egui::RawInput {
            events,
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(320.0, 400.0))),
            ..Default::default()
        };
        let mut run = run;
        ctx.run_ui(input, |ui| run(ui))
            .drop_without_applying_deltas();
    }

    #[test]
    fn a_section_opens_and_closes_from_its_whole_bar() {
        let ctx = egui::Context::default();
        let shown = std::cell::Cell::new(false);
        let body = |ui: &mut Ui| {
            shown.set(false);
            let _ = section(ui, "t", "Geometry", false, |ui| {
                shown.set(true);
                ui.label("row");
            });
        };
        frame(&ctx, Vec::new(), body);
        assert!(!shown.get(), "closed by default");
        // A click far right on the bar, not on the chevron, opens it.
        let at = pos2(250.0, ROW_HEIGHT / 2.0);
        let click = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&ctx, vec![egui::Event::PointerMoved(at), click(true)], body);
        frame(&ctx, vec![click(false)], body);
        ctx.memory_mut(|m| m.everything_is_visible());
        for _ in 0..30 {
            frame(&ctx, Vec::new(), body);
        }
        assert!(shown.get(), "the click opened it");
    }

    #[test]
    fn a_changed_mark_dots_only_the_next_row() {
        let ctx = egui::Context::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            mark_next_changed(ui, true);
            prop_row(ui, "Changed", |ui| ui.label("a"));
            prop_row(ui, "Same", |ui| ui.label("b"));
            mark_next_changed(ui, false);
            prop_row(ui, "Also same", |ui| ui.label("c"));
        });
        let dots = output
            .shapes
            .iter()
            .filter(|s| matches!(&s.shape, egui::Shape::Circle(_)))
            .count();
        output.drop_without_applying_deltas();
        assert_eq!(dots, 1);
    }

    #[test]
    fn the_chevron_turns_a_quarter_as_it_opens() {
        let ctx = egui::Context::default();
        let mut closed = Vec::new();
        let mut open = Vec::new();
        ctx.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.painter();
            chevron(painter, egui::Pos2::ZERO, 0.0, egui::Color32::WHITE);
            chevron(painter, egui::Pos2::ZERO, 1.0, egui::Color32::WHITE);
        })
        .drop_without_applying_deltas();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.painter();
            chevron(painter, egui::Pos2::ZERO, 0.0, egui::Color32::WHITE);
            chevron(painter, egui::Pos2::ZERO, 1.0, egui::Color32::WHITE);
        });
        let shapes = output.shapes.clone();
        output.drop_without_applying_deltas();
        for (n, shape) in shapes.iter().enumerate() {
            if let egui::Shape::Path(path) = &shape.shape {
                if n == 0 {
                    closed.clone_from(&path.points);
                } else {
                    open.clone_from(&path.points);
                }
            }
        }
        // Closed: the tip points right (+x); open: it points down (+y).
        assert!(closed[1].x > closed[0].x && (closed[1].y).abs() < 1e-3);
        assert!(open[1].y > open[0].y && (open[1].x).abs() < 1e-3);
    }

    #[test]
    fn a_value_wider_than_its_cell_wraps_and_a_short_one_keeps_one_row() {
        let ctx = egui::Context::default();
        let long = std::cell::Cell::new(0.0_f32);
        let short = std::cell::Cell::new(0.0_f32);
        frame(&ctx, Vec::new(), |ui| {
            prop_value(
                ui,
                "Volume",
                "2861833.1892138589999999999999999999",
                Some("mm³"),
            );
            long.set(ui.min_rect().height());
        });
        frame(&ctx, Vec::new(), |ui| {
            prop_value(ui, "Mass", "39.3", Some("kg"));
            short.set(ui.min_rect().height());
        });
        // The wide value wraps onto a second line inside its cell instead of running left
        // over the label column; the short one keeps the one-row height.
        assert!(
            long.get() >= short.get() + 10.0,
            "the wide value wrapped onto a second line"
        );
        assert!(short.get() <= 2.0 * ROW_HEIGHT, "one row for a short value");
    }
}
