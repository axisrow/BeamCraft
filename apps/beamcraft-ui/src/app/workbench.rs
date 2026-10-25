// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The layout of the model window (design v1): a brand header with the project and Publish, a
//! ribbon of tools, the 3D view with its view and zoom controls, an inspector on the right that
//! shows what the chosen tool needs, and a status bar. The panels it shows run the same commands
//! as before; tools that do not exist yet are shown disabled, with a tooltip saying so.

use super::App;
use crate::edit::EditKind;
use crate::publish::Phase;
use crate::theme::{self, BRAND, INK, Icon, Kind, Palette};
use crate::tools::{MemberKind, ToolKind};
use egui::{Rect, RichText, Sense, Stroke, StrokeKind, Ui, vec2};

/// What the inspector shows: the tool chosen on the ribbon.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Bench {
    /// Select elements and edit their properties.
    #[default]
    Select,
    /// Create a plate.
    Plate,
    /// Create a beam.
    Beam,
    /// Create a column.
    Column,
    /// Move the selected element.
    Move,
    /// Copy the selected element.
    Copy,
    /// Delete the selected element.
    Delete,
}

impl Bench {
    /// The inspector's title.
    pub(crate) const fn title(self) -> &'static str {
        match self {
            Self::Select => "Properties",
            Self::Plate => "New plate",
            Self::Beam => "New beam",
            Self::Column => "New column",
            Self::Move => "Move",
            Self::Copy => "Copy",
            Self::Delete => "Delete",
        }
    }

    /// The edit action of a Move, Copy or Delete bench.
    pub(crate) const fn edit(self) -> Option<EditKind> {
        match self {
            Self::Move => Some(EditKind::Move),
            Self::Copy => Some(EditKind::Copy),
            Self::Delete => Some(EditKind::Delete),
            Self::Select | Self::Plate | Self::Beam | Self::Column => None,
        }
    }
}

/// Why a ribbon tool is disabled, shown as its tooltip.
const LATER: &str = "Not available yet";

/// One ribbon button: an icon over a label. Active tools are brand cyan with an ink border.
fn tool_button(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    active: bool,
    enabled: bool,
) -> egui::Response {
    let palette = Palette::current(ui);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(62.0, 56.0), sense);
    let painter = ui.painter();
    let radius = egui::CornerRadius::same(9);
    if active {
        painter.rect_filled(rect.translate(vec2(2.0, 2.0)), radius, palette.ink);
        painter.rect_filled(rect, radius, BRAND);
        painter.rect_stroke(rect, radius, Stroke::new(2.0, INK), StrokeKind::Inside);
    } else if enabled && response.hovered() {
        painter.rect_filled(rect, radius, palette.head);
    }
    let alpha = if enabled { 1.0 } else { 0.35 };
    let line = if active { INK } else { palette.line };
    let icon_rect = Rect::from_center_size(rect.center() - vec2(0.0, 8.0), vec2(28.0, 28.0));
    // A disabled icon is painted faint on a separate layer of the same painter.
    let mut faint = painter.clone();
    faint.multiply_opacity(alpha);
    theme::paint_icon(&faint, icon_rect, icon, line);
    let text = if active { INK } else { palette.text };
    faint.text(
        rect.center_bottom() - vec2(0.0, 8.0),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(11.0),
        text,
    );
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response.on_hover_text(LATER)
    }
}

/// A small group caption under ribbon buttons.
fn caption(ui: &mut Ui, text: &str) {
    let palette = Palette::current(ui);
    ui.label(
        RichText::new(text)
            .size(10.0)
            .family(theme::bold_family())
            .color(palette.muted),
    );
}

impl App {
    /// Switches the inspector to `bench` and points the create or edit forms at it.
    pub(crate) fn set_bench(&mut self, bench: Bench) {
        self.bench = bench;
        match bench {
            Bench::Plate => self.tools.active = ToolKind::Plate,
            Bench::Beam => {
                self.tools.active = ToolKind::Member;
                self.tools.member.kind = MemberKind::Beam;
            }
            Bench::Column => {
                self.tools.active = ToolKind::Member;
                self.tools.member.kind = MemberKind::Column;
            }
            Bench::Move | Bench::Copy | Bench::Delete => {
                if let Some(kind) = bench.edit() {
                    self.editor.set_action(kind);
                }
            }
            Bench::Select => {}
        }
    }

    /// The brand header: logo, project, save state, environment and author, Publish.
    pub(crate) fn header(&mut self, ui: &mut Ui) {
        let palette = Palette::current(ui);
        egui::Panel::top("model-header")
            .exact_size(52.0)
            .frame(
                egui::Frame::new()
                    .fill(BRAND)
                    .inner_margin(egui::Margin::symmetric(14, 0)),
            )
            .show(ui, |ui| {
                let bottom = ui.max_rect().bottom();
                ui.horizontal_centered(|ui| {
                    if self.logo.is_none() && !self.logo_tried {
                        self.logo_tried = true;
                        self.logo = theme::load_logo(ui.ctx(), 96);
                    }
                    theme::logo(ui, self.logo.as_ref(), 32.0);
                    let name = self
                        .source
                        .store
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("Model")
                        .to_owned();
                    ui.label(theme::bold(name, 15.0).color(INK));
                    let saving = self.other_command_running() || self.reload_job.is_some();
                    let state = if saving {
                        "Saving…"
                    } else {
                        "All changes saved"
                    };
                    // A pill as tall as its text, painted: a frame here would take the
                    // header's whole height.
                    let galley = ui.painter().layout_no_wrap(
                        state.to_owned(),
                        egui::FontId::new(12.0, theme::bold_family()),
                        INK,
                    );
                    let (pill, _) =
                        ui.allocate_exact_size(galley.size() + vec2(20.0, 8.0), Sense::hover());
                    let radius = egui::CornerRadius::same(255);
                    ui.painter().rect_filled(pill, radius, theme::CREAM);
                    ui.painter().rect_stroke(
                        pill,
                        radius,
                        Stroke::new(1.5, INK),
                        StrokeKind::Inside,
                    );
                    ui.painter()
                        .galley(pill.center() - galley.size() / 2.0, galley, INK);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let target = self.publisher.target();
                        let enabled = target.is_ok()
                            && self.publisher.phase() == Phase::Idle
                            && !self.other_command_running();
                        let response = theme::button(ui, Kind::Secondary, "Publish", enabled);
                        let response = match &target {
                            Err(why) => response.on_disabled_hover_text(why.to_string()),
                            Ok(_) => response,
                        };
                        if response.clicked()
                            && let Err(why) = self.publisher.request()
                        {
                            self.publisher.note = Some(Err(why.into()));
                        }
                        let who = [self.source.environment.clone(), self.source.author.clone()]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join(" · ");
                        if !who.is_empty() {
                            ui.label(theme::bold(who, 13.0).color(INK));
                        }
                    });
                });
                ui.painter().line_segment(
                    [
                        egui::pos2(ui.max_rect().left() - 14.0, bottom),
                        egui::pos2(ui.max_rect().right() + 14.0, bottom),
                    ],
                    Stroke::new(2.0, palette.ink),
                );
            });
    }

    /// The ribbon of tools.
    pub(crate) fn ribbon(&mut self, ui: &mut Ui) {
        let palette = Palette::current(ui);
        egui::Panel::top("model-ribbon")
            .exact_size(86.0)
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .inner_margin(egui::Margin::symmetric(10, 4)),
            )
            .show(ui, |ui| {
                let bottom = ui.max_rect().bottom() + 4.0;
                let selected = self.editor.selected().is_some();
                let edits_free = !self.editor.busy() && !self.other_command_running();
                let has_workspace = self.source.workspace.is_some();
                let mut pick = None;
                let (mut undo, mut redo) = (false, false);
                ui.horizontal(|ui| {
                    let group = |ui: &mut Ui, name: &str, add: &mut dyn FnMut(&mut Ui)| {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing = vec2(4.0, 2.0);
                            ui.horizontal(|ui| add(ui));
                            caption(ui, name);
                        });
                        ui.separator();
                    };
                    let bench = self.bench;
                    group(ui, "SELECT", &mut |ui| {
                        if tool_button(ui, Icon::Select, "Select", bench == Bench::Select, true)
                            .clicked()
                        {
                            pick = Some(Bench::Select);
                        }
                    });
                    group(ui, "STEEL", &mut |ui| {
                        for (b, icon, label) in [
                            (Bench::Plate, Icon::Plate, "Plate"),
                            (Bench::Beam, Icon::Beam, "Beam"),
                            (Bench::Column, Icon::Column, "Column"),
                        ] {
                            if tool_button(ui, icon, label, bench == b, true).clicked() {
                                pick = Some(b);
                            }
                        }
                    });
                    group(ui, "CONNECT", &mut |ui| {
                        tool_button(ui, Icon::Bolt, "Bolts", false, false);
                        tool_button(ui, Icon::Weld, "Weld", false, false);
                    });
                    group(ui, "EDIT", &mut |ui| {
                        for (b, icon, label) in [
                            (Bench::Move, Icon::Move, "Move"),
                            (Bench::Copy, Icon::Copy, "Copy"),
                            (Bench::Delete, Icon::Delete, "Delete"),
                        ] {
                            let response = tool_button(ui, icon, label, bench == b, selected);
                            if response.clicked() {
                                pick = Some(b);
                            }
                        }
                        let can = has_workspace && edits_free;
                        undo = tool_button(ui, Icon::Undo, "Undo", false, can).clicked();
                        redo = tool_button(ui, Icon::Redo, "Redo", false, can).clicked();
                    });
                    group(ui, "DISPLAY", &mut |ui| {
                        tool_button(ui, Icon::Block, "Shaded", true, true);
                        tool_button(ui, Icon::Wire, "Wire", false, false);
                    });
                });
                if let Some(bench) = pick {
                    self.set_bench(bench);
                }
                if undo {
                    self.start_edit(Some(EditKind::Undo));
                }
                if redo {
                    self.start_edit(Some(EditKind::Redo));
                }
                ui.painter().line_segment(
                    [
                        egui::pos2(ui.max_rect().left() - 10.0, bottom),
                        egui::pos2(ui.max_rect().right() + 10.0, bottom),
                    ],
                    Stroke::new(2.0, palette.ink),
                );
            });
    }

    /// The inspector: what the chosen tool needs, then the publishing state.
    pub(crate) fn inspector(&mut self, ui: &mut Ui) {
        let palette = Palette::current(ui);
        egui::Panel::right("model-inspector")
            .exact_size(320.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .inner_margin(egui::Margin::symmetric(16, 14)),
            )
            .show(ui, |ui| {
                let left = ui.max_rect().left() - 16.0;
                ui.painter().line_segment(
                    [
                        egui::pos2(left, ui.max_rect().top() - 14.0),
                        egui::pos2(left, ui.max_rect().bottom() + 14.0),
                    ],
                    Stroke::new(2.0, palette.ink),
                );
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // The inspector keeps its width: long values and ids wrap instead.
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        ui.set_max_width(ui.available_width());
                        ui.label(theme::bold(self.bench.title(), 18.0));
                        match self.bench {
                            Bench::Select => self.props_panel(ui),
                            Bench::Plate | Bench::Beam | Bench::Column => self.tools_panel(ui),
                            Bench::Move | Bench::Copy | Bench::Delete => self.edit_panel(ui),
                        }
                        ui.add_space(12.0);
                        egui::CollapsingHeader::new("Publishing")
                            .default_open(false)
                            .show(ui, |ui| self.publish_panel(ui));
                    });
            });
    }

    /// The status bar: the selection, snap switches (not available yet) and Details.
    pub(crate) fn status_bar(&mut self, ui: &mut Ui) {
        let palette = Palette::current(ui);
        egui::Panel::bottom("model-status")
            .exact_size(34.0)
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .inner_margin(egui::Margin::symmetric(14, 0)),
            )
            .show(ui, |ui| {
                let top = ui.max_rect().top();
                ui.painter().line_segment(
                    [
                        egui::pos2(ui.max_rect().left() - 14.0, top),
                        egui::pos2(ui.max_rect().right() + 14.0, top),
                    ],
                    Stroke::new(2.0, palette.ink),
                );
                ui.horizontal_centered(|ui| {
                    let selection = match self.editor.selected() {
                        Some(info) => format!("1 selected · {} {}", info.kind, info.name),
                        None => "Nothing selected".to_owned(),
                    };
                    ui.label(theme::bold(selection, 12.0));
                    if self.select_job.is_some() {
                        ui.label(RichText::new("Looking up…").small().color(palette.muted));
                    }
                    if let Some(error) = &self.error {
                        ui.label(RichText::new(error).small().color(palette.error));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .selectable_label(self.details, RichText::new("Details").small())
                            .clicked()
                        {
                            self.details = !self.details;
                        }
                        for snap in ["Nearest", "Grid", "Middle", "End"] {
                            ui.add_enabled(false, egui::Button::new(RichText::new(snap).small()))
                                .on_disabled_hover_text(LATER);
                        }
                        ui.label(RichText::new("Snap").small().color(palette.muted));
                    });
                });
            });
    }

    /// The Details window: the adapter and frame rate, the revision on screen and the
    /// navigation settings.
    pub(crate) fn details_window(&mut self, ctx: &egui::Context) {
        let mut open = self.details;
        egui::Window::new("Details")
            .open(&mut open)
            .resizable(false)
            .default_width(360.0)
            .show(ctx, |ui| self.info(ui));
        self.details = open;
    }

    /// The view and zoom controls drawn over the 3D view at `rect`.
    pub(crate) fn view_controls(&mut self, ui: &mut Ui, rect: Rect) {
        let palette = Palette::current(ui);
        let at = Rect::from_min_size(rect.min + vec2(14.0, 14.0), vec2(320.0, 50.0));
        let mut bar = ui.new_child(egui::UiBuilder::new().max_rect(at));
        theme::card(palette, 3)
            .inner_margin(egui::Margin::same(4))
            .show(&mut bar, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Button::new(theme::bold("3D", 12.0).color(INK))
                            .fill(BRAND)
                            .stroke(Stroke::new(2.0, INK)),
                    );
                    for view in ["Plan", "Grid A"] {
                        ui.add_enabled(false, egui::Button::new(RichText::new(view).small()))
                            .on_disabled_hover_text(LATER);
                    }
                });
            });
        let at = Rect::from_min_size(rect.right_bottom() - vec2(78.0, 160.0), vec2(64.0, 146.0));
        let mut zoom = ui.new_child(egui::UiBuilder::new().max_rect(at));
        theme::card(palette, 3)
            .inner_margin(egui::Margin::same(4))
            .show(&mut zoom, |ui| {
                ui.vertical_centered(|ui| {
                    let small = |ui: &mut Ui, text: &str, tip: &str| {
                        ui.add(
                            egui::Button::new(theme::bold(text, 16.0)).min_size(vec2(36.0, 34.0)),
                        )
                        .on_hover_text(tip)
                        .clicked()
                    };
                    if small(ui, "+", "Zoom in (+)") {
                        self.pending_zoom = Some(true);
                    }
                    if small(ui, "−", "Zoom out (-)") {
                        self.pending_zoom = Some(false);
                    }
                    if small(ui, "Fit", "Fit the model (F)") {
                        self.pending_fit = true;
                    }
                });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::Bench;
    use crate::edit::EditKind;

    #[test]
    fn edit_benches_map_to_edit_actions() {
        assert_eq!(Bench::Move.edit(), Some(EditKind::Move));
        assert_eq!(Bench::Copy.edit(), Some(EditKind::Copy));
        assert_eq!(Bench::Delete.edit(), Some(EditKind::Delete));
        for bench in [Bench::Select, Bench::Plate, Bench::Beam, Bench::Column] {
            assert_eq!(bench.edit(), None);
        }
        assert_eq!(Bench::default(), Bench::Select);
    }
}
