// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The native model workspace: a neutral header with the project and Publish, a
//! ribbon of tools, the 3D view with its view and zoom controls, an inspector on the right that
//! shows what the chosen tool needs, and a status bar. The panels it shows run the same commands
//! as before; tools that do not exist yet are shown disabled, with a tooltip saying so.

use super::App;
use crate::edit::EditKind;
use crate::publish::Phase;
use crate::theme::{self, Icon, Palette};
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
    /// Create a rectangular project grid.
    Grid,
    /// Move the selected element.
    Move,
    /// Copy the selected element.
    Copy,
    /// Delete the selected element.
    Delete,
    /// The window's display cut plane (V15); a view filter, never a model change.
    CutPlane,
    /// Colour by a saved representation set (V17); a display overlay, never a model change.
    Colour,
    /// D06 annotations: place dimensions, marks and leaders, watch their binding states.
    Annotations,
}

impl Bench {
    /// The inspector's title.
    pub(crate) const fn title(self) -> &'static str {
        match self {
            Self::Select => "Properties",
            Self::Plate => "New plate",
            Self::Beam => "New beam",
            Self::Column => "New column",
            Self::Grid => "Rectangular grid",
            Self::Move => "Move",
            Self::Copy => "Copy",
            Self::Delete => "Delete",
            Self::CutPlane => "Cut plane",
            Self::Colour => "Colour",
            Self::Annotations => "Annotations",
        }
    }

    /// The edit action of a Move, Copy or Delete bench.
    pub(crate) const fn edit(self) -> Option<EditKind> {
        match self {
            Self::Move => Some(EditKind::Move),
            Self::Copy => Some(EditKind::Copy),
            Self::Delete => Some(EditKind::Delete),
            Self::Select
            | Self::Plate
            | Self::Beam
            | Self::Column
            | Self::Grid
            | Self::CutPlane
            | Self::Colour
            | Self::Annotations => None,
        }
    }
}

/// Why a ribbon tool is disabled, shown as its tooltip.
const LATER: &str = "Not available yet";

/// The command families shown on the ribbon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RibbonTab {
    Steel,
    Edit,
    View,
    Manage,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum MenuSection {
    Import,
    Export,
    Settings,
    #[default]
    Catalogs,
    Editors,
    Diagnose,
    Help,
    Extend,
    Profiles,
    Materials,
    About,
}

impl MenuSection {
    fn heading(self) -> (&'static str, &'static str) {
        match self {
            Self::Import => ("Import", "Bring model data into the workspace"),
            Self::Export => ("Export", "Export model data"),
            Self::Settings => ("Settings", "Application and workspace preferences"),
            Self::Catalogs => ("Catalogs", "Profiles, materials and components"),
            Self::Editors => ("Editors", "Templates, reports and custom components"),
            Self::Diagnose => ("Diagnose & repair", "Model checks and maintenance"),
            Self::Help => ("Help", "Navigation and application information"),
            Self::Extend => ("Extend", "Application extensions"),
            Self::Profiles => (
                "Profile catalog",
                "Profiles in the open model's definitions",
            ),
            Self::Materials => (
                "Material catalog",
                "Materials in the open model's definitions",
            ),
            Self::About => ("About BeamCraft", "Native structural steel detailing"),
        }
    }

    fn commands(self) -> &'static [&'static str] {
        match self {
            Self::Import => &["BeamCraft file...", "IFC", "DWG / DXF"],
            Self::Export => &[
                "BeamCraft file",
                "IFC",
                "DWG / DXF",
                "glTF / GLB",
                "NC files",
            ],
            Self::Settings => &["Options", "Advanced options", "Keyboard shortcuts"],
            Self::Catalogs => &[
                "Profile catalog",
                "Material catalog",
                "Bolt catalog",
                "Bolt assembly catalog",
                "Shape catalog",
                "Component catalog",
            ],
            Self::Editors => &[
                "Template editor",
                "Report editor",
                "Custom component editor",
            ],
            Self::Diagnose => &["Diagnose model", "Repair model"],
            Self::Help => &["Help", "About BeamCraft"],
            Self::Extend => &["Extensions", "Extension manager"],
            Self::Profiles | Self::Materials | Self::About => &[],
        }
    }
}

fn menu_navigation(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    active: bool,
    enabled: bool,
) -> egui::Response {
    let palette = Palette::current(ui);
    let response = ui.add_enabled(
        enabled,
        egui::Button::new("")
            .min_size(vec2(ui.available_width(), 34.0))
            .fill(if active {
                palette.side_grid
            } else {
                egui::Color32::TRANSPARENT
            })
            .stroke(Stroke::NONE)
            .corner_radius(4),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    if !active && enabled && response.hovered() {
        ui.painter()
            .rect_filled(response.rect, 4, palette.side_grid);
    }
    let color = if enabled {
        palette.side_text()
    } else {
        palette.side_text().gamma_multiply(0.45)
    };
    theme::paint_icon(
        ui.painter(),
        Rect::from_center_size(
            response.rect.left_center() + vec2(20.0, 0.0),
            vec2(18.0, 18.0),
        ),
        icon,
        color,
    );
    ui.painter().text(
        response.rect.left_center() + vec2(44.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.0),
        color,
    );
    response.on_disabled_hover_text(LATER)
}

fn menu_command(ui: &mut Ui, label: &str, enabled: bool) -> egui::Response {
    let palette = Palette::current(ui);
    let response = ui.add_enabled(
        enabled,
        egui::Button::new("")
            .min_size(vec2(ui.available_width(), 42.0))
            .frame(false),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    ui.painter().text(
        response.rect.left_center() + vec2(12.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.0),
        if enabled { palette.text } else { palette.muted },
    );
    ui.painter().text(
        response.rect.right_center() - vec2(12.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        "→",
        egui::FontId::proportional(14.0),
        palette.muted,
    );
    response.on_disabled_hover_text(LATER)
}

/// One keyboard-accessible ribbon button with a contributor-original glyph.
fn tool_button(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    active: bool,
    enabled: bool,
) -> egui::Response {
    let palette = Palette::current(ui);
    let label_width = ui
        .painter()
        .layout_no_wrap(
            label.to_owned(),
            egui::FontId::proportional(11.0),
            palette.text,
        )
        .size()
        .x;
    let response = ui.add_enabled(
        enabled,
        egui::Button::new("")
            .min_size(vec2((label_width + 12.0).max(54.0), 64.0))
            .fill(if active {
                palette.select
            } else {
                egui::Color32::TRANSPARENT
            })
            .stroke(Stroke::NONE)
            .corner_radius(4),
    );
    let rect = response.rect;
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    let painter = ui.painter();
    let radius = egui::CornerRadius::same(4);
    if active {
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, palette.rule),
            StrokeKind::Inside,
        );
    }
    let alpha = if enabled { 1.0 } else { 0.35 };
    let line = palette.line;
    let icon_rect = Rect::from_center_size(rect.center() - vec2(0.0, 10.0), vec2(24.0, 24.0));
    // A disabled icon is painted faint on a separate layer of the same painter.
    let mut faint = painter.clone();
    faint.multiply_opacity(alpha);
    theme::paint_icon(&faint, icon_rect, icon, line);
    let text = palette.text;
    faint.text(
        rect.center_bottom() - vec2(0.0, 13.0),
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
    ui.label(RichText::new(text).size(10.0).color(palette.muted));
}

impl App {
    /// Switches the inspector to `bench` and points the create or edit forms at it.
    pub(crate) fn set_bench(&mut self, bench: Bench) {
        if self.other_command_running() {
            return;
        }
        self.grid_line.click = None;
        self.tools.cancel();
        if bench != Bench::Select && self.grid_edit.take().is_some() {
            self.grid_form = crate::grid::GridForm::default();
            self.grid_note = None;
        }
        self.plate_mode = bench == Bench::Plate;
        self.plate_draft.clear();
        self.tools.plate_plane = None;
        self.distance_input.clear();
        self.distance_aim = None;
        self.plate_hover = None;
        self.tools.plate.contour.clear();
        if self.tools.plate.thickness.is_empty() {
            self.tools.plate.thickness = "10".into();
        }
        self.beam_mode = matches!(bench, Bench::Beam | Bench::Column);
        self.beam.reset();
        self.ghost_end = None;
        self.beam_hover = None;
        self.renderer
            .set_ghost(&self.state.device, &self.state.queue, None);
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
            Bench::CutPlane => {}
            Bench::Colour => {}
            Bench::Annotations => {}
            Bench::Grid => {
                self.beam_mode = false;
                self.beam.reset();
                self.ghost_end = None;
                self.renderer
                    .set_ghost(&self.state.device, &self.state.queue, None);
            }
        }
        if bench == Bench::Annotations {
            self.annotation_load();
        }
    }

    /// The reference-sized title bar: logo, quick history, project and saved state.
    pub(crate) fn header(&mut self, ui: &mut Ui) {
        let palette = Palette::current(ui);
        egui::Panel::top("model-header")
            .exact_size(36.0)
            .frame(
                egui::Frame::new()
                    .fill(palette.head)
                    .inner_margin(egui::Margin::symmetric(10, 0)),
            )
            .show(ui, |ui| {
                theme::titlebar_drag(ui);
                ui.horizontal_centered(|ui| {
                    if self.logo.is_none() && !self.logo_tried {
                        self.logo_tried = true;
                        self.logo = theme::load_logo(ui.ctx(), 96);
                    }
                    theme::logo(ui, self.logo.as_ref(), 20.0);
                    let free = !self.other_command_running() && !self.editor.busy();
                    if !self.menu_open {
                        if theme::icon_button(
                            ui,
                            Icon::Undo,
                            "Undo",
                            self.source.workspace.is_some() && free,
                        )
                        .clicked()
                        {
                            self.start_edit(Some(EditKind::Undo));
                        }
                        if theme::icon_button(
                            ui,
                            Icon::Redo,
                            "Redo",
                            self.source.workspace.is_some() && free,
                        )
                        .clicked()
                        {
                            self.start_edit(Some(EditKind::Redo));
                        }
                        ui.separator();
                    }
                    let name = self
                        .source
                        .store
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("Model")
                        .to_owned();
                    ui.label(RichText::new(name).size(12.0).color(palette.text));
                    ui.label(
                        RichText::new(if self.menu_open {
                            "— Menu"
                        } else {
                            "— BeamCraft"
                        })
                        .size(11.0)
                        .color(palette.muted),
                    );
                    let saving = self.other_command_running() || self.reload_job.is_some();
                    let state = if saving { "Saving…" } else { "Saved locally" };
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        theme::window_controls(ui);
                        if let Some(appearance) = &mut self.appearance {
                            appearance.error_ui(ui, true);
                        }
                        if self.menu_open {
                            return;
                        }
                        ui.separator();
                        ui.label(RichText::new(state).size(11.0).color(palette.muted));
                        let (dot, _) = ui.allocate_exact_size(vec2(6.0, 6.0), Sense::hover());
                        ui.painter().circle_filled(
                            dot.center(),
                            2.5,
                            if saving {
                                palette.muted
                            } else {
                                egui::Color32::from_rgb(0x36, 0x78, 0x55)
                            },
                        );
                        if self.publisher.target().is_ok()
                            && ui
                                .add_enabled(
                                    free && self.publisher.phase() == Phase::Idle,
                                    egui::Button::new("Publish").small(),
                                )
                                .clicked()
                            && let Err(why) = self.publisher.request()
                        {
                            self.publisher.note = Some(Err(why.into()));
                        }
                    });
                });
            });
    }

    /// The ribbon of tools.
    pub(crate) fn ribbon(&mut self, ui: &mut Ui) {
        let palette = Palette::current(ui);
        egui::Panel::top("ribbon-families")
            .exact_size(36.0)
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .inner_margin(egui::Margin::symmetric(12, 0)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    if ui
                        .add(
                            egui::Button::new("Menu")
                                .frame(false)
                                .min_size(vec2(64.0, 32.0)),
                        )
                        .clicked()
                    {
                        self.menu_open = true;
                    }
                    for (tab, label) in [
                        (RibbonTab::Steel, "Steel"),
                        (RibbonTab::Edit, "Edit"),
                        (RibbonTab::View, "View"),
                        (RibbonTab::Manage, "Manage"),
                    ] {
                        let response = ui.add(
                            egui::Button::new(label)
                                .frame(false)
                                .min_size(vec2(56.0, 32.0)),
                        );
                        if self.ribbon_tab == tab {
                            ui.painter().line_segment(
                                [response.rect.left_bottom(), response.rect.right_bottom()],
                                Stroke::new(2.0, palette.text),
                            );
                        }
                        if response.clicked() {
                            self.ribbon_tab = tab;
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("Find a command")
                                        .size(11.0)
                                        .color(palette.muted),
                                )
                                .frame(false),
                            )
                            .clicked()
                        {
                            self.command_search = true;
                            self.command_focus = true;
                        }
                    });
                });
            });
        egui::Panel::top("model-ribbon")
            .exact_size(106.0)
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .inner_margin(egui::Margin::symmetric(14, 8)),
            )
            .show(ui, |ui| {
                let bottom = ui.max_rect().bottom() + 4.0;
                let grid_selected = self.grid_edit.is_some();
                let selected = self.editor.selected().is_some() && !grid_selected;
                let edits_free = !self.editor.busy() && !self.other_command_running();
                let has_workspace = self.source.workspace.is_some();
                let mut pick = None;
                let (mut undo, mut redo) = (false, false);
                egui::ScrollArea::horizontal().id_salt("ribbon-tools").show(ui, |ui| {
                ui.horizontal(|ui| {
                    let group = |ui: &mut Ui, name: &str, add: &mut dyn FnMut(&mut Ui)| {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing = vec2(4.0, 14.0);
                            let row = ui.horizontal(|ui| add(ui));
                            ui.allocate_ui_with_layout(vec2(row.response.rect.width(), 12.0),
                                egui::Layout::top_down(egui::Align::Center), |ui| caption(ui, name));
                        });
                        ui.separator();
                    };
                    let bench = self.bench;
                    if matches!(self.ribbon_tab, RibbonTab::Steel | RibbonTab::Edit) {
                    group(ui, "Select", &mut |ui| {
                        if tool_button(ui, Icon::Select, "Select", bench == Bench::Select, edits_free)
                            .clicked()
                        {
                            pick = Some(Bench::Select);
                        }
                    });
                    }
                    if self.ribbon_tab == RibbonTab::Steel {
                    group(ui, "Members", &mut |ui| {
                        for (b, icon, label) in [
                            (Bench::Column, Icon::Column, "Column"),
                            (Bench::Beam, Icon::Beam, "Beam"),
                            (Bench::Plate, Icon::Plate, "Plate"),
                        ] {
                            if tool_button(ui, icon, label, bench == b, edits_free).clicked() {
                                pick = Some(b);
                            }
                        }
                        tool_button(ui, Icon::Beam, "Brace", false, false);
                    });
                    group(ui, "Connect", &mut |ui| {
                        tool_button(ui, Icon::Bolt, "Bolts", false, false);
                        tool_button(ui, Icon::Weld, "Weld", false, false);
                    });
                    }
                    if self.ribbon_tab == RibbonTab::Edit {
                    group(ui, "Modify", &mut |ui| {
                        for (b, icon, label) in [
                            (Bench::Move, Icon::Move, "Move"),
                            (Bench::Copy, Icon::Copy, "Copy"),
                            (Bench::Delete, Icon::Delete, "Delete"),
                        ] {
                            let response = tool_button(ui, icon, label, bench == b, (selected || b == Bench::Delete && grid_selected) && edits_free);
                            if response.clicked() {
                                pick = Some(b);
                            }
                        }
                    });
                    group(ui, "History", &mut |ui| {
                        let can = has_workspace && edits_free;
                        undo = tool_button(ui, Icon::Undo, "Undo", false, can).clicked();
                        redo = tool_button(ui, Icon::Redo, "Redo", false, can).clicked();
                    });
                    }
                    if self.ribbon_tab == RibbonTab::View {
                    group(ui, "Views", &mut |ui| {
                        if tool_button(ui, Icon::Block, "Saved views", self.views_panel.window_open, true).clicked() {
                            self.views_panel.window_open = !self.views_panel.window_open;
                        }
                        if tool_button(ui, Icon::Copy, "New view window", false, self.view_windows.len() < crate::views::MAX_WINDOWS).clicked() {
                            self.open_view_window();
                        }
                    });
                    group(ui, "Display", &mut |ui| {
                        for (mode, label) in [
                            (beamcraft_engine::RenderMode::Wireframe, "Wireframe"),
                            (beamcraft_engine::RenderMode::Transparent, "Transparent"),
                            (beamcraft_engine::RenderMode::Grey, "Grey"),
                            (beamcraft_engine::RenderMode::Shaded, "Shaded"),
                            (beamcraft_engine::RenderMode::SelectedOnly, "Selected only"),
                        ] {
                            if tool_button(ui, Icon::Block, label, self.mode == mode, true).clicked() {
                                self.mode = mode;
                            }
                        }
                        if tool_button(ui, Icon::Grid, "Grid guides", self.grids_visible, true).clicked() {
                            self.grids_visible = !self.grids_visible;
                            if !self.grids_visible { self.grid_edit = None; }
                        }
                    });
                    group(ui, "Colour", &mut |ui| {
                        if tool_button(ui, Icon::Block, "Colour", self.bench == Bench::Colour, true)
                            .clicked()
                        {
                            pick = Some(Bench::Colour);
                        }
                    });
                    group(ui, "Cut", &mut |ui| {
                        if tool_button(ui, Icon::Block, "Cut plane", self.bench == Bench::CutPlane, true).clicked() {
                            pick = Some(Bench::CutPlane);
                        }
                    });
                    group(ui, "Refresh", &mut |ui| {
                        if tool_button(ui, Icon::Refresh, "Reload", false, true).clicked() {
                            self.reload();
                        }
                    });
                    }
                    if matches!(self.ribbon_tab, RibbonTab::Steel | RibbonTab::View) {
                        group(ui, "Inspect", &mut |ui| {
                            tool_button(ui, Icon::Plate, "Measure", false, false);
                            if tool_button(ui, Icon::Fit, "Fit model", false, true).clicked() {
                                self.pending_fit = true;
                            }
                        });
                    }
                    if self.ribbon_tab == RibbonTab::Manage {
                        group(ui, "Grids", &mut |ui| {
                            if tool_button(ui, Icon::Grid, "Rectangular grid", bench == Bench::Grid, true).clicked() {
                                pick = Some(Bench::Grid);
                            }
                            if tool_button(ui, Icon::Callout, "Annotations", bench == Bench::Annotations, true).clicked() {
                                pick = Some(Bench::Annotations);
                            }
                        });
                        ui.vertical(|ui| {
                            ui.label(theme::bold("Workspace", 13.0));
                            ui.label("Publish from the header; inspect revision and navigation in Details.");
                            if ui.button("Details").clicked() {
                                self.details = true;
                            }
                        });
                    }
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
                        egui::pos2(ui.max_rect().left() - 10.0, bottom),
                        egui::pos2(ui.max_rect().right() + 10.0, bottom),
                    ],
                    Stroke::new(1.0, palette.rule),
                );
            });
    }

    /// The inspector: what the chosen tool needs, then the publishing state.
    pub(crate) fn inspector(&mut self, ui: &mut Ui) {
        let palette = Palette::current(ui);
        egui::Panel::right("model-inspector")
            .default_size(286.0)
            .size_range(250.0..=(ui.available_width() * 0.75).max(320.0))
            .resizable(true)
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .inner_margin(egui::Margin::symmetric(14, 12)),
            )
            .show(ui, |ui| {
                let left = ui.max_rect().left() - 16.0;
                ui.painter().line_segment(
                    [
                        egui::pos2(left, ui.max_rect().top() - 14.0),
                        egui::pos2(left, ui.max_rect().bottom() + 14.0),
                    ],
                    Stroke::new(1.0, palette.rule),
                );
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(if self.grid_edit.is_some() {
                            "Grid properties"
                        } else {
                            self.bench.title()
                        })
                        .size(12.0),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.bench == Bench::Select
                            && theme::icon_button(
                                ui,
                                Icon::Refresh,
                                "Reload properties",
                                self.props.selected().is_some()
                                    && !self.other_command_running()
                                    && !self.props.busy(),
                            )
                            .clicked()
                        {
                            self.reread();
                        }
                    });
                });
                ui.separator();
                if self.bench == Bench::Grid
                    || self.bench == Bench::Select && self.grid_edit.is_some()
                {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                    self.grid_panel(ui);
                    return;
                }
                if self.bench == Bench::Select && self.props.selected().is_some() {
                    egui::Panel::bottom("property-actions")
                        .exact_size(54.0)
                        .frame(
                            egui::Frame::new()
                                .fill(palette.surface)
                                .inner_margin(egui::Margin::symmetric(0, 8)),
                        )
                        .show(ui, |ui| self.property_actions(ui));
                }
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // The inspector keeps its width: long values and ids wrap instead.
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        ui.set_max_width(ui.available_width());
                        match self.bench {
                            Bench::Select => self.props_panel(ui),
                            Bench::Plate | Bench::Beam | Bench::Column => self.tools_panel(ui),
                            Bench::Grid => self.grid_panel(ui),
                            Bench::Move | Bench::Copy | Bench::Delete => self.edit_panel(ui),
                            Bench::CutPlane => self.cut_panel(ui),
                            Bench::Colour => self.colour_panel(ui),
                            Bench::Annotations => self.annotation_panel(ui),
                        }
                        ui.add_space(12.0);
                        egui::CollapsingHeader::new("Publishing")
                            .default_open(false)
                            .show(ui, |ui| self.publish_panel(ui));
                        egui::CollapsingHeader::new("Sheet editor")
                            .default_open(false)
                            .show(ui, |ui| {
                                ui.label(
                                    "Move the placed views of a drawing document on its sheet.",
                                );
                                if ui.button("Open sheet editor").clicked() {
                                    self.sheet_editor.open = true;
                                }
                            });
                    });
            });
    }

    /// The status bar: the selection, snap switches (not available yet) and Details.
    pub(crate) fn status_bar(&mut self, ui: &mut Ui) {
        let palette = Palette::current(ui);
        egui::Panel::bottom("model-status")
            .exact_size(52.0)
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .inner_margin(egui::Margin::symmetric(6, 3)),
            )
            .show(ui, |ui| {
                ui.spacing_mut().interact_size.y = 16.0;
                ui.spacing_mut().item_spacing.y = 2.0;
                let top = ui.max_rect().top();
                ui.painter().line_segment(
                    [
                        egui::pos2(ui.max_rect().left() - 6.0, top - 3.0),
                        egui::pos2(ui.max_rect().right() + 6.0, top - 3.0),
                    ],
                    Stroke::new(1.0, palette.rule),
                );
                ui.horizontal(|ui| {
                    if super::switches::entities(
                        ui,
                        self.bench == Bench::Select,
                        &mut self.selection,
                    ) {
                        self.set_bench(Bench::Select);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.allocate_ui_with_layout(
                            vec2(super::switches::snaps_width(), 24.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| super::switches::snaps(ui, &mut self.selection),
                        );
                    });
                });
                ui.horizontal(|ui| {
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
                    ui.label(RichText::new(selection).size(10.0).color(palette.muted));
                    if self.select_job.is_some() {
                        ui.label(RichText::new("Looking up…").small().color(palette.muted));
                    }
                    if let Some(error) = &self.error {
                        ui.label(RichText::new(error).small().color(palette.error));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .selectable_label(
                                self.selection.snapping_enabled,
                                RichText::new("Snap").size(10.0),
                            )
                            .clicked()
                        {
                            self.selection.snapping_enabled = !self.selection.snapping_enabled;
                        }
                        if ui
                            .selectable_label(self.details, RichText::new("Details").size(10.0))
                            .clicked()
                        {
                            self.details = !self.details;
                        }
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

    fn command_list(&mut self, ui: &mut Ui, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        let editable = self.editor.selected().is_some() && !self.other_command_running();
        for (label, bench) in [
            ("Select", Bench::Select),
            ("Column", Bench::Column),
            ("Rectangular grid", Bench::Grid),
            ("Beam", Bench::Beam),
            ("Plate", Bench::Plate),
            ("Move", Bench::Move),
            ("Copy", Bench::Copy),
            ("Delete", Bench::Delete),
        ] {
            if label.to_lowercase().contains(&query)
                && ui
                    .add_enabled(
                        bench.edit().is_none()
                            || editable
                            || bench == Bench::Delete
                                && self.grid_edit.is_some()
                                && !self.other_command_running(),
                        egui::Button::new(label).min_size(vec2(ui.available_width(), 34.0)),
                    )
                    .clicked()
            {
                if bench == Bench::Delete && self.grid_edit.is_some() {
                    self.delete_selection();
                } else {
                    self.set_bench(bench);
                }
                return true;
            }
        }
        for label in [
            "Undo",
            "Redo",
            "Fit model",
            "Reload model",
            "Details",
            "Publish",
            "Model library",
        ] {
            let enabled = match label {
                "Undo" | "Redo" => self.source.workspace.is_some() && !self.other_command_running(),
                "Publish" => {
                    self.publisher.target().is_ok()
                        && !self.other_command_running()
                        && self.publisher.phase() == Phase::Idle
                }
                "Model library" => self.from_library && !self.other_command_running(),
                _ => true,
            };
            if label.to_lowercase().contains(&query)
                && ui
                    .add_enabled(
                        enabled,
                        egui::Button::new(label).min_size(vec2(ui.available_width(), 34.0)),
                    )
                    .clicked()
            {
                match label {
                    "Undo" => self.start_edit(Some(EditKind::Undo)),
                    "Redo" => self.start_edit(Some(EditKind::Redo)),
                    "Fit model" => self.pending_fit = true,
                    "Reload model" => self.reload(),
                    "Details" => self.details = true,
                    "Model library" => self.return_to_library = true,
                    "Publish" => {
                        if let Err(why) = self.publisher.request() {
                            self.publisher.note = Some(Err(why.into()));
                        }
                    }
                    _ => {}
                }
                return true;
            }
        }
        false
    }

    /// Full native application menu, using the same real actions as the ribbon.
    pub(crate) fn menu_page(&mut self, ui: &mut Ui) {
        let palette = Palette::current(ui);
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.menu_open = false;
        }
        egui::Panel::left("application-menu-navigation")
            .exact_size(232.0)
            .frame(egui::Frame::new().fill(palette.side).inner_margin(12))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height((ui.available_height() - 84.0).max(0.0))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        if menu_navigation(ui, Icon::Back, "Back to model", false, true).clicked() {
                            self.menu_open = false;
                        }
                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            ui.add_space(12.0);
                            ui.label(theme::bold("Menu", 23.0).color(palette.side_text()));
                        });
                        ui.add_space(14.0);
                        let free = self.from_library && !self.other_command_running();
                        if menu_navigation(ui, Icon::Plus, "New", false, free).clicked() {
                            self.return_to_create = true;
                            self.return_to_library = true;
                        }
                        if menu_navigation(ui, Icon::Folder, "Open", false, free).clicked() {
                            self.return_to_library = true;
                        }
                        menu_navigation(ui, Icon::Copy, "Save as", false, false);
                        menu_navigation(ui, Icon::Folder, "Open model folder", false, false);
                        for (section, icon, label) in [
                            (MenuSection::Import, Icon::Back, "Import"),
                            (MenuSection::Export, Icon::Shared, "Export"),
                        ] {
                            if menu_navigation(ui, icon, label, self.menu_section == section, true)
                                .clicked()
                            {
                                self.menu_section = section;
                            }
                        }
                        if menu_navigation(ui, Icon::Block, "Project properties", false, true)
                            .clicked()
                        {
                            self.details = true;
                            self.menu_open = false;
                        }
                        for (section, icon, label) in [
                            (MenuSection::Settings, Icon::Grid, "Settings"),
                            (MenuSection::Catalogs, Icon::Wire, "Catalogs"),
                            (MenuSection::Editors, Icon::Plate, "Editors"),
                        ] {
                            let active = self.menu_section == section
                                || (section == MenuSection::Catalogs
                                    && matches!(
                                        self.menu_section,
                                        MenuSection::Profiles | MenuSection::Materials
                                    ));
                            if menu_navigation(ui, icon, label, active, true).clicked() {
                                self.menu_section = section;
                            }
                        }
                        menu_navigation(ui, Icon::Recent, "Logs", false, false);
                        for (section, icon, label) in [
                            (MenuSection::Diagnose, Icon::Fit, "Diagnose & repair"),
                            (MenuSection::Help, Icon::Recent, "Help"),
                            (MenuSection::Extend, Icon::Plus, "Extend"),
                        ] {
                            if menu_navigation(ui, icon, label, self.menu_section == section, true)
                                .clicked()
                            {
                                self.menu_section = section;
                            }
                        }
                        if menu_navigation(
                            ui,
                            Icon::Back,
                            "Exit BeamCraft",
                            false,
                            !self.other_command_running(),
                        )
                        .clicked()
                        {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                ui.add_space((ui.available_height() - 64.0).max(16.0));
                ui.label(
                    RichText::new(
                        self.source
                            .store
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("Model"),
                    )
                    .size(12.0)
                    .color(palette.side_text()),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(self.source.store.display().to_string())
                            .size(10.0)
                            .color(palette.side_text()),
                    )
                    .wrap(),
                );
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(palette.page)
                    .inner_margin(egui::Margin::symmetric(40, 36)),
            )
            .show(ui, |ui| {
                let (title, description) = self.menu_section.heading();
                ui.label(theme::bold(title, 28.0));
                ui.label(RichText::new(description).size(12.0).color(palette.muted));
                ui.add_space(30.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.set_width(ui.available_width().min(620.0));
                    ui.separator();
                    ui.add_space(15.0);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for label in self.menu_section.commands() {
                        let enabled = matches!(
                            *label,
                            "Profile catalog" | "Material catalog" | "About BeamCraft"
                        );
                        if menu_command(ui, label, enabled).clicked() {
                            match *label {
                                "Profile catalog" => self.menu_section = MenuSection::Profiles,
                                "Material catalog" => self.menu_section = MenuSection::Materials,
                                "About BeamCraft" => {
                                    self.menu_section = MenuSection::About;
                                }
                                _ => {}
                            }
                        }
                    }
                    let definitions = match self.menu_section {
                        MenuSection::Profiles => Some(&self.tools.definitions.profiles),
                        MenuSection::Materials => Some(&self.tools.definitions.materials),
                        _ => None,
                    };
                    if let Some(definitions) = definitions {
                        if let Some(error) = &self.refresh_error {
                            ui.colored_label(palette.error, error.to_string());
                        } else if definitions.is_empty() {
                            ui.label("No definitions in this model.");
                        }
                        for definition in definitions {
                            ui.add_sized(
                                vec2(ui.available_width(), 42.0),
                                egui::Label::new(&definition.label),
                            );
                        }
                    }
                    if self.menu_section == MenuSection::About {
                        ui.label(theme::bold(
                            format!("BeamCraft {}", env!("CARGO_PKG_VERSION")),
                            16.0,
                        ));
                        ui.add_space(12.0);
                        ui.label("Developed by Apibim.");
                        ui.label("Native Rust interface with egui and wgpu.");
                        ui.label("Project code: MPL-2.0. BeamCraft brand rights are reserved.");
                    }
                });
            });
    }

    /// Search only the commands supported by the native command client.
    pub(crate) fn command_dialog(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::K)) {
            self.command_search = true;
            self.command_focus = true;
        }
        if self.command_search
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.command_search = false;
        }
        if !self.command_search {
            return;
        }
        let mut open = true;
        egui::Window::new("Find a command")
            .open(&mut open)
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .anchor(egui::Align2::CENTER_TOP, vec2(0.0, 80.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(theme::bold("Find a command", 14.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("×").on_hover_text("Close search").clicked() {
                            self.command_search = false;
                        }
                    });
                });
                let search = ui.add(
                    egui::TextEdit::singleline(&mut self.command_query)
                        .hint_text("Search commands")
                        .desired_width(ui.available_width())
                        .char_limit(128),
                );
                if self.command_focus {
                    search.request_focus();
                    self.command_focus = false;
                }
                let query = self.command_query.clone();
                egui::ScrollArea::vertical()
                    .max_height(380.0)
                    .show(ui, |ui| {
                        if self.command_list(ui, &query) {
                            self.command_search = false;
                        }
                    });
            });
        self.command_search &= open;
    }

    /// The project browser shows only views and data actually available in the engine snapshot.
    pub(crate) fn project_browser(&mut self, ui: &mut Ui) {
        let palette = Palette::current(ui);
        egui::Panel::left("project-browser")
            .default_size(214.0)
            .min_size(160.0)
            .max_size(300.0)
            .resizable(true)
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .inner_margin(egui::Margin::symmetric(10, 12)),
            )
            .show(ui, |ui| {
                ui.label(RichText::new("Project browser").size(12.0));
                ui.separator();
                ui.add_space(6.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.browser_search)
                        .hint_text("Search views and sheets")
                        .desired_width(ui.available_width())
                        .font(egui::FontId::proportional(11.0))
                        .char_limit(128),
                );
                ui.add_space(12.0);
                let name = self
                    .source
                    .store
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Model");
                ui.horizontal(|ui| {
                    theme::icon(ui, Icon::Block, 16.0, palette.text);
                    ui.label(RichText::new(name).size(12.0));
                });
                ui.add_space(10.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let needle = self.browser_search.trim().to_lowercase();
                    if needle.is_empty() || "views 3d model".contains(&needle) {
                        // The named views of the model (V12): clicking restores the view; the
                        // {3D} entry fits the whole model. Actions run after the panel closes.
                        let mut pick = None;
                        egui::CollapsingHeader::new("Views (all)")
                            .default_open(true)
                            .show(ui, |ui| {
                                ui.add_enabled(false, egui::Label::new("Floor Plans"))
                                    .on_disabled_hover_text(LATER);
                                egui::CollapsingHeader::new("3D Views")
                                    .default_open(true)
                                    .show(ui, |ui| {
                                        if ui.selectable_label(true, "{3D}").clicked() {
                                            self.pending_fit = true;
                                        }
                                        for view in self.views.iter() {
                                            let kind = match &view.plane {
                                                None => "3D",
                                                Some(
                                                    beamcraft_engine::views::PlaneSpec::GridLine {
                                                        ..
                                                    },
                                                ) => "grid line",
                                                Some(beamcraft_engine::views::PlaneSpec::Level) => {
                                                    "level"
                                                }
                                                Some(
                                                    beamcraft_engine::views::PlaneSpec::TwoPoints {
                                                        ..
                                                    },
                                                ) => "two points",
                                            };
                                            ui.horizontal(|ui| {
                                                if ui.selectable_label(false, &view.name).clicked()
                                                {
                                                    pick = Some(crate::views::Action::Apply(
                                                        view.name.clone(),
                                                    ));
                                                }
                                                ui.label(
                                                    RichText::new(kind)
                                                        .size(10.0)
                                                        .color(palette.muted),
                                                );
                                                if ui
                                                    .button("×")
                                                    .on_hover_text("Remove view")
                                                    .clicked()
                                                {
                                                    pick = Some(crate::views::Action::Delete(
                                                        view.name.clone(),
                                                    ));
                                                }
                                            });
                                        }
                                    });
                                for label in ["Elevations", "Sections", "Drafting Views"] {
                                    ui.add_enabled(false, egui::Label::new(label))
                                        .on_disabled_hover_text(LATER);
                                }
                            });
                        if let Some(action) = pick {
                            self.view_action(action);
                        }
                    }
                    for title in ["Schedules / Quantities", "Sheets (all)"] {
                        if needle.is_empty() || title.to_lowercase().contains(&needle) {
                            egui::CollapsingHeader::new(title).show(ui, |ui| {
                                ui.label(
                                    RichText::new("Not available in the native UI yet")
                                        .size(10.0)
                                        .color(palette.muted),
                                );
                            });
                        }
                    }
                    egui::CollapsingHeader::new("Model information").show(ui, |ui| {
                        if let Some(loaded) = &self.loaded {
                            ui.label(format!("Elements: {}", loaded.summary.elements));
                            ui.label(format!("Triangles: {}", loaded.summary.triangles));
                        } else {
                            ui.label("No snapshot loaded");
                        }
                        ui.label("Units: mm");
                    });
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
