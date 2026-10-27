// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The layout of the model window (design v1): a brand header with the project and Publish, a
//! tabbed tool ribbon, an unobstructed 3D view, an inspector on the right that
//! shows what the chosen tool needs, and a status bar. The panels it shows run the same commands
//! as before; tools that do not exist yet are shown disabled, with a tooltip saying so.

use super::App;
use crate::edit::EditKind;
use crate::publish::Phase;
use crate::selection::Selection;
use crate::theme::{self, BRAND, INK, Icon, Kind, Palette};
use crate::tools::{MemberKind, ToolKind};
use beamcraft_engine::query::pick::{SelectionClass, SnapKind};
use egui::{Rect, RichText, Sense, Stroke, StrokeKind, Ui, vec2};

/// The panel label of a snap kind.
fn snap_label(kind: SnapKind) -> &'static str {
    match kind {
        SnapKind::Vertex => "Vertex",
        SnapKind::AxisEnd => "Axis end",
        SnapKind::Axis => "Axis (nearest point)",
        SnapKind::GridIntersection => "Grid intersection",
        SnapKind::GridEnd => "Grid end",
        SnapKind::GridLine => "Grid line (nearest point)",
    }
}

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
    /// Insert a regular project grid.
    Grid,
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
            Self::Grid => "New grid",
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
            Self::Select | Self::Plate | Self::Beam | Self::Column | Self::Grid => None,
        }
    }
}

/// The visible command category; switching it changes only the UI.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum RibbonTab {
    #[default]
    Model,
    Modify,
    View,
    Drawings,
    Project,
}

impl RibbonTab {
    const ALL: [Self; 5] = [
        Self::Model,
        Self::Modify,
        Self::View,
        Self::Drawings,
        Self::Project,
    ];

    const fn title(self) -> &'static str {
        match self {
            Self::Model => "Model",
            Self::Modify => "Modify",
            Self::View => "View",
            Self::Drawings => "Drawings",
            Self::Project => "Project",
        }
    }
}

/// Why a ribbon tool is disabled, shown as its tooltip.
const LATER: &str = "Not available yet";

/// Stable identity for the painted workspace-menu toggle and its input tests.
pub(super) const MENU_TOGGLE_ID: &str = "workspace-menu-toggle";
/// Stable identity for the adjacent selection tool.
pub(super) const SELECT_TOOL_ID: &str = "selection-tool";
const SELECT_BUTTON_SIZE: egui::Vec2 = vec2(44.0, 56.0);
const MENU_RAIL_SIZE: egui::Vec2 = vec2(32.0, 86.0);

/// Three geometric strokes, independent of the font's rendering of menu symbols.
fn menu_toggle(ui: &mut Ui, open: bool) -> egui::Response {
    let (_, rect) = ui.allocate_space(MENU_RAIL_SIZE);
    let response = ui.interact(rect, egui::Id::unique(MENU_TOGGLE_ID), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, true, open, "Workspace menu")
    });
    ui.painter().rect_filled(
        rect,
        0,
        if response.hovered() || open {
            theme::BRAND_LIGHT
        } else {
            BRAND
        },
    );
    let centre = rect.center();
    for offset in [-6.0, 0.0, 6.0] {
        ui.painter().line_segment(
            [centre + vec2(-9.0, offset), centre + vec2(9.0, offset)],
            Stroke::new(2.0, INK),
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(2.0),
            0,
            Stroke::new(1.0, INK),
            StrokeKind::Inside,
        );
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Workspace menu")
}

/// A text tab joined to the ribbon by an underline, without a button frame.
fn command_tab(ui: &mut Ui, label: &str, selected: bool) -> egui::Response {
    let palette = Palette::current(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(96.0, 38.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, selected, label));
    if selected || response.hovered() || response.has_focus() {
        ui.painter().rect_filled(
            rect,
            0,
            if selected {
                palette.surface
            } else {
                palette.select
            },
        );
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::new(13.0, theme::bold_family()),
        if selected {
            palette.text
        } else {
            palette.muted
        },
    );
    if selected {
        ui.painter().line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            Stroke::new(3.0, BRAND),
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

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
    let compact = label.is_empty();
    let (rect, response) = if compact {
        let (_, rect) = ui.allocate_space(SELECT_BUTTON_SIZE);
        (
            rect,
            ui.interact(rect, egui::Id::unique(SELECT_TOOL_ID), sense),
        )
    } else {
        ui.allocate_exact_size(vec2(62.0, 56.0), sense)
    };
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::Button,
            enabled,
            if compact { "Select" } else { label },
        )
    });
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
    let icon_rect = Rect::from_center_size(
        rect.center() - vec2(0.0, if compact { 0.0 } else { 8.0 }),
        vec2(28.0, 28.0),
    );
    // A disabled icon is painted faint on a separate layer of the same painter.
    let mut faint = painter.clone();
    faint.multiply_opacity(alpha);
    theme::paint_icon(&faint, icon_rect, icon, line);
    let text = if active { INK } else { palette.text };
    if !compact {
        faint.text(
            rect.center_bottom() - vec2(0.0, 8.0),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(11.0),
            text,
        );
    }
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
        if bench != Bench::Select && self.grid_edit.take().is_some() {
            self.grid_form = crate::grid::GridForm::default();
            self.grid_note = None;
        }
        self.plane_pick = false;
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
            Bench::Grid => {
                self.beam_mode = false;
                self.beam.reset();
                self.ghost_end = None;
                self.renderer
                    .set_ghost(&self.state.device, &self.state.queue, None);
            }
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
        egui::Panel::top("model-command-tabs")
            .exact_size(40.0)
            .frame(
                egui::Frame::new()
                    .fill(palette.head)
                    .inner_margin(egui::Margin::symmetric(10, 0)),
            )
            .show(ui, |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("command-tabs-scroll")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for tab in RibbonTab::ALL {
                                if command_tab(ui, tab.title(), self.ribbon_tab == tab).clicked() {
                                    self.ribbon_tab = tab;
                                }
                            }
                        });
                    });
            });
        egui::Panel::top("model-ribbon")
            .exact_size(86.0)
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .inner_margin(egui::Margin {
                        left: 0,
                        right: 10,
                        top: 0,
                        bottom: 0,
                    }),
            )
            .show(ui, |ui| {
                let bottom = ui.max_rect().bottom();
                let grid_selected = self.grid_edit.is_some();
                let selected = self.editor.selected().is_some() && !grid_selected;
                let edits_free = !self.editor.busy() && !self.other_command_running();
                let has_workspace = self.source.workspace.is_some();
                let mut pick = None;
                let (mut undo, mut redo) = (false, false);
                ui.horizontal(|ui| {
                    if menu_toggle(ui, self.menu_open).clicked() {
                        self.menu_open = !self.menu_open;
                    }
                    let bench = self.bench;
                    if tool_button(ui, Icon::Select, "", bench == Bench::Select, true)
                        .on_hover_text("Select (Esc)")
                        .clicked()
                    {
                        pick = Some(Bench::Select);
                    }
                    ui.separator();
                    egui::ScrollArea::horizontal()
                        .id_salt("ribbon-tools-scroll")
                        .show(ui, |ui| {
                            ui.horizontal_top(|ui| {
                                let group =
                                    |ui: &mut Ui, name: &str, add: &mut dyn FnMut(&mut Ui)| {
                                        ui.vertical(|ui| {
                                            ui.spacing_mut().item_spacing = vec2(4.0, 2.0);
                                            ui.horizontal(|ui| add(ui));
                                            caption(ui, name);
                                        });
                                        ui.separator();
                                    };
                                match self.ribbon_tab {
                                    RibbonTab::Model => {
                                        group(ui, "STEEL PARTS", &mut |ui| {
                                            for (b, icon, label) in [
                                                (Bench::Plate, Icon::Plate, "Plate"),
                                                (Bench::Beam, Icon::Beam, "Beam"),
                                                (Bench::Column, Icon::Column, "Column"),
                                            ] {
                                                if tool_button(ui, icon, label, bench == b, true)
                                                    .clicked()
                                                {
                                                    pick = Some(b);
                                                }
                                            }
                                        });
                                        group(ui, "SHAPES", &mut |ui| {
                                            for label in
                                                ["Polybeam", "Curved", "Contour", "Bent plate"]
                                            {
                                                tool_button(ui, Icon::Block, label, false, false);
                                            }
                                        });
                                        group(ui, "CONNECT", &mut |ui| {
                                            tool_button(ui, Icon::Bolt, "Bolts", false, false);
                                            tool_button(ui, Icon::Weld, "Weld", false, false);
                                            tool_button(ui, Icon::Block, "Assembly", false, false);
                                        });
                                        group(ui, "LAYOUT", &mut |ui| {
                                            if tool_button(
                                                ui,
                                                Icon::Grid,
                                                "Grid",
                                                bench == Bench::Grid,
                                                true,
                                            )
                                            .clicked()
                                            {
                                                pick = Some(Bench::Grid);
                                            }
                                            tool_button(ui, Icon::Plus, "Guide", false, false);
                                        });
                                    }
                                    RibbonTab::Modify => {
                                        group(ui, "EDIT", &mut |ui| {
                                            for (b, icon, label) in [
                                                (Bench::Move, Icon::Move, "Move"),
                                                (Bench::Copy, Icon::Copy, "Copy"),
                                                (Bench::Delete, Icon::Delete, "Delete"),
                                            ] {
                                                let response = tool_button(
                                                    ui,
                                                    icon,
                                                    label,
                                                    bench == b,
                                                    (selected
                                                        || b == Bench::Delete && grid_selected)
                                                        && edits_free,
                                                );
                                                if response.clicked() {
                                                    pick = Some(b);
                                                }
                                            }
                                            let can = has_workspace && edits_free;
                                            undo = tool_button(ui, Icon::Undo, "Undo", false, can)
                                                .clicked();
                                            redo = tool_button(ui, Icon::Redo, "Redo", false, can)
                                                .clicked();
                                        });
                                        group(ui, "TRANSFORM", &mut |ui| {
                                            for label in ["Rotate", "Mirror", "Array"] {
                                                tool_button(ui, Icon::Move, label, false, false);
                                            }
                                        });
                                        group(ui, "SHAPE EDITS", &mut |ui| {
                                            for label in
                                                ["Trim", "Cut", "Split", "Combine", "Chamfer"]
                                            {
                                                tool_button(ui, Icon::Plate, label, false, false);
                                            }
                                        });
                                    }
                                    RibbonTab::View => {
                                        group(ui, "DISPLAY", &mut |ui| {
                                            tool_button(ui, Icon::Block, "Shaded", true, true);
                                            tool_button(ui, Icon::Wire, "Wire", false, false);
                                        });
                                        group(ui, "VIEWS", &mut |ui| {
                                            for label in
                                                ["Plan", "Elevation", "Section", "Saved views"]
                                            {
                                                tool_button(ui, Icon::Wire, label, false, false);
                                            }
                                        });
                                        group(ui, "VISIBILITY", &mut |ui| {
                                            tool_button(ui, Icon::Block, "Filters", false, false);
                                            tool_button(ui, Icon::Block, "Colours", false, false);
                                        });
                                    }
                                    RibbonTab::Drawings => {
                                        group(ui, "DOCUMENTS", &mut |ui| {
                                            tool_button(ui, Icon::Plus, "New sheet", false, false);
                                            tool_button(ui, Icon::Folder, "Drawings", false, false);
                                        });
                                        group(ui, "ANNOTATION", &mut |ui| {
                                            for label in ["Dimension", "Mark", "Text"] {
                                                tool_button(ui, Icon::Plus, label, false, false);
                                            }
                                        });
                                        group(ui, "OUTPUT", &mut |ui| {
                                            tool_button(ui, Icon::Plate, "Print", false, false);
                                            tool_button(
                                                ui,
                                                Icon::Plate,
                                                "Export PDF",
                                                false,
                                                false,
                                            );
                                        });
                                    }
                                    RibbonTab::Project => {
                                        group(ui, "PROJECT DATA", &mut |ui| {
                                            tool_button(
                                                ui,
                                                Icon::Block,
                                                "Information",
                                                false,
                                                false,
                                            );
                                            tool_button(ui, Icon::Recent, "History", false, false);
                                        });
                                        group(ui, "LIBRARIES", &mut |ui| {
                                            tool_button(ui, Icon::Beam, "Profiles", false, false);
                                            tool_button(ui, Icon::Block, "Materials", false, false);
                                            tool_button(ui, Icon::Bolt, "Fasteners", false, false);
                                        });
                                        group(ui, "PREFERENCES", &mut |ui| {
                                            tool_button(ui, Icon::Select, "Settings", false, false);
                                            tool_button(
                                                ui,
                                                Icon::Select,
                                                "Shortcuts",
                                                false,
                                                false,
                                            );
                                        });
                                    }
                                }
                            });
                        });
                });
                if let Some(bench) = pick {
                    if bench == Bench::Delete && grid_selected {
                        self.delete_selection();
                    } else {
                        self.set_bench(bench);
                    }
                }
                if undo {
                    self.start_edit(Some(EditKind::Undo));
                }
                if redo {
                    self.start_edit(Some(EditKind::Redo));
                }
                ui.painter().line_segment(
                    [
                        egui::pos2(ui.max_rect().left(), bottom),
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
            .default_size(320.0)
            .size_range(280.0..=(ui.available_width() * 0.75).max(320.0))
            .resizable(true)
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
                if self.bench == Bench::Grid
                    || self.bench == Bench::Select && self.grid_edit.is_some()
                {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                    ui.label(theme::bold(
                        if self.grid_edit.is_some() {
                            "Grid properties"
                        } else {
                            self.bench.title()
                        },
                        18.0,
                    ));
                    self.grid_panel(ui);
                    return;
                }
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // Long values wrap within the width chosen by the user.
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        ui.set_max_width(ui.available_width());
                        ui.label(theme::bold(self.bench.title(), 18.0));
                        match self.bench {
                            Bench::Select => self.props_panel(ui),
                            Bench::Plate | Bench::Beam | Bench::Column => self.tools_panel(ui),
                            Bench::Grid => self.grid_panel(ui),
                            Bench::Move | Bench::Copy | Bench::Delete => self.edit_panel(ui),
                        }
                        ui.add_space(12.0);
                        egui::CollapsingHeader::new("Publishing")
                            .default_open(false)
                            .show(ui, |ui| self.publish_panel(ui));
                    });
            });
    }

    /// The status bar: the selection, the selection switches, the Snap switch and Details.
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
                    let selection = if let Some(grid) = self
                        .grid_edit
                        .as_ref()
                        .and_then(|edit| self.grid_scene.get(&edit.id))
                    {
                        format!("1 selected · Grid {}", grid.form.name)
                    } else {
                        match self.editor.selected() {
                            Some(info) => format!("1 selected · {} {}", info.kind, info.name),
                            None => "Nothing selected".to_owned(),
                        }
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
                        if ui
                            .selectable_label(
                                self.selection.snap_open,
                                RichText::new("Snap").small(),
                            )
                            .on_hover_text("Snap switches")
                            .clicked()
                        {
                            self.selection.snap_open = !self.selection.snap_open;
                        }
                        // The selection switches (V11): a switch that is off takes its class
                        // out of picking, and nothing else — never the model or the snapshot
                        // on screen.
                        for class in SelectionClass::ALL.iter().rev() {
                            let on = self.selection.is_on(*class);
                            if ui
                                .selectable_label(
                                    on,
                                    RichText::new(Selection::label(*class)).small(),
                                )
                                .on_hover_text(Selection::tip(*class))
                                .clicked()
                            {
                                self.selection.set(*class, !on);
                            }
                        }
                    });
                });
            });
    }

    /// The snap strip above the status bar: which snap kinds the tools' point resolution asks
    /// the engine's `snap` for, while the bar's Snap switch is on. Session state only; the
    /// tools that resolve click points through it arrive with T01.6.
    pub(crate) fn snap_panel(&mut self, ui: &mut Ui) {
        if !self.selection.snap_open {
            return;
        }
        let palette = Palette::current(ui);
        egui::Panel::bottom("snap-panel")
            .exact_size(30.0)
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
                    ui.label(
                        RichText::new("Snap")
                            .small()
                            .family(theme::bold_family())
                            .color(palette.muted),
                    );
                    for kind in SnapKind::ALL {
                        let mut on = self.selection.snap_on(kind);
                        if ui.checkbox(&mut on, snap_label(kind)).changed() {
                            self.selection.set_snap(kind, on);
                        }
                    }
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
}

#[cfg(test)]
mod tests {
    use super::Bench;
    use crate::edit::EditKind;

    #[test]
    fn the_menu_icon_is_three_lines_and_contains_no_font_glyph() {
        let ctx = egui::Context::default();
        let mut rect = egui::Rect::NOTHING;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            rect = super::menu_toggle(ui, false).rect;
        });
        // This layout test has no painter; deliberately discard texture uploads.
        output.textures_delta.clear();
        let lines = output
            .shapes
            .iter()
            .filter(|shape| {
                matches!(
                    &shape.shape,
                    egui::Shape::LineSegment { points: [a, b], .. }
                        if a.y == b.y && a.x < b.x && rect.contains(*a) && rect.contains(*b)
                )
            })
            .count();
        assert_eq!(lines, 3);
        assert!(
            !output
                .shapes
                .iter()
                .any(|shape| matches!(shape.shape, egui::Shape::Text(_)))
        );
    }

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
