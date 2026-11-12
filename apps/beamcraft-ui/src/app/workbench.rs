// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The native model workspace: a neutral header with the project and Publish, a
//! ribbon of tools, the 3D view with its view and zoom controls, an inspector on the right that
//! shows what the chosen tool needs, and a status bar. The panels it shows run the same commands
//! as before; tools that do not exist yet are shown disabled, with a tooltip saying so.

use super::App;
use super::work_area::Arrange;
use crate::edit::EditKind;
use crate::publish::Phase;
use crate::theme::{self, Icon, Kind, Palette};
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
    /// Create a fillet weld between two picked parts (V10); the A06 create-weld command.
    Weld,
    /// Place a bolt group of a library fastener between picked parts (V19); the A14.3
    /// create-bolt-group command.
    Bolt,
    /// D06 annotations: place dimensions, marks and leaders, watch their binding states.
    Annotations,
    /// A view's properties, or a new view.
    View,
    /// The project's levels: create, edit and delete.
    Levels,
}

/// Height the application menu's navigation needs with its "Menu" heading: fourteen 34 pt
/// entries one point apart plus the heading and its spacing.
const MENU_FULL_HEIGHT: f32 = 14.0 * 35.0 + 60.0;
/// Width of each chevron that scrolls the status bar's snap strip when it does not fit.
const SNAP_CHEVRON_WIDTH: f32 = 16.0;
/// How far one chevron click scrolls the snap strip: about three switches.
const SNAP_CHEVRON_STEP: f32 = 72.0;

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
            Self::Weld => "Weld",
            Self::Bolt => "Bolt group",
            Self::Annotations => "Annotations",
            Self::View => "View properties",
            Self::Levels => "Levels",
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
            | Self::Weld
            | Self::Bolt
            | Self::Annotations
            | Self::View
            | Self::Levels => None,
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
    New,
    Open,
    Import,
    Export,
    Project,
    Settings,
    #[default]
    Catalogs,
    Editors,
    Help,
    Profiles,
    Materials,
    About,
}

impl MenuSection {
    /// The section a sub-page belongs to; `None` for a top-level section.
    const fn parent(self) -> Option<Self> {
        match self {
            Self::Profiles | Self::Materials => Some(Self::Catalogs),
            Self::About => Some(Self::Help),
            _ => None,
        }
    }

    /// The navigation row that is highlighted for this page.
    const fn top(self) -> Self {
        match self.parent() {
            Some(parent) => parent,
            None => self,
        }
    }

    fn heading(self) -> (&'static str, &'static str) {
        match self {
            Self::New => ("New model", "Create a model from a template and open it"),
            Self::Open => ("Open model", "Recent models, or any model folder"),
            Self::Import => ("Import", "Bring model data into the workspace"),
            Self::Export => ("Export", "Export model data"),
            Self::Project => (
                "Project properties",
                "The open model's project and this session",
            ),
            Self::Settings => ("Settings", "Application and workspace preferences"),
            Self::Catalogs => ("Catalogs", "Profiles, materials and components"),
            Self::Editors => ("Editors", "Templates, reports and custom components"),
            Self::Help => ("Help", "Navigation and application information"),
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
            Self::Help => &["Help", "About BeamCraft"],
            Self::New
            | Self::Open
            | Self::Project
            | Self::Profiles
            | Self::Materials
            | Self::About => &[],
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
    tool_button_inner(ui, icon, label, active, enabled, false)
}

/// A ribbon button that opens `items` in a popup under it; no default action.
fn dropdown_button(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    enabled: bool,
    items: impl FnOnce(&mut Ui),
) -> egui::Response {
    let response = tool_button_inner(ui, icon, label, false, enabled, true);
    egui::Popup::menu(&response).show(items);
    response
}

/// The ribbon button behind [`tool_button`] and [`dropdown_button`]; `chevron` adds a small
/// down-pointing triangle right of the caption.
fn tool_button_inner(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    active: bool,
    enabled: bool,
    chevron: bool,
) -> egui::Response {
    let palette = Palette::current(ui);
    let extra = if chevron { 12.0 } else { 0.0 };
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
            .min_size(vec2((label_width + 12.0).max(54.0) + extra, 64.0))
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
    // With a chevron the caption shifts left by half its width, so the two stay centred.
    let caption_at = rect.center_bottom() - vec2(extra / 2.0, 13.0);
    faint.text(
        caption_at,
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(11.0),
        text,
    );
    if chevron {
        let left = caption_at.x + label_width / 2.0 + 4.0;
        faint.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(left, caption_at.y - 2.0),
                egui::pos2(left + 6.0, caption_at.y - 2.0),
                egui::pos2(left + 3.0, caption_at.y + 2.0),
            ],
            line,
            Stroke::NONE,
        ));
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
        if self.tools.plate.thickness_side.is_empty() {
            self.tools.plate.thickness_side = "top".into();
        }
        self.beam_mode = matches!(bench, Bench::Beam | Bench::Column);
        self.beam.reset();
        self.ghost_end = None;
        self.beam_hover = None;
        self.clear_ghost();
        self.plane_pick = false;
        self.bench = bench;
        // The view panel's pick and subject never outlive the panel.
        if bench != Bench::View {
            self.views.picking = None;
            self.views.subject = crate::views::Subject::default();
        }
        match bench {
            Bench::Plate => {
                self.tools.active = ToolKind::Plate;
                self.tools.use_preset("plate", None);
            }
            Bench::Beam => {
                self.tools.active = ToolKind::Member;
                self.tools.member.kind = MemberKind::Beam;
                self.tools.use_preset("beam", None);
            }
            Bench::Column => {
                self.tools.active = ToolKind::Member;
                self.tools.member.kind = MemberKind::Column;
                self.tools.use_preset("column", None);
            }
            Bench::Move | Bench::Copy | Bench::Delete => {
                if let Some(kind) = bench.edit() {
                    self.editor.set_action(kind);
                }
            }
            Bench::Select => {}
            Bench::CutPlane => {}
            Bench::Colour => {}
            Bench::Weld => {}
            Bench::Bolt => self.bolt_load(),
            Bench::Annotations => {}
            Bench::View => {}
            Bench::Grid | Bench::Levels => {
                self.beam_mode = false;
                self.beam.reset();
                self.ghost_end = None;
                self.clear_ghost();
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
                    theme::native_controls_inset(ui);
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
                    // Saving only while something writes; the read-only work (a catalogue
                    // read, the annotations list, a reload of what is already written) is
                    // not saving and says so.
                    let saving = self.writing_to_model();
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
                        // A search field look-alike that opens the command search; the shortcut
                        // sits at its end, as a CAD command line advertises it.
                        let shortcut = ui.ctx().format_shortcut(&egui::KeyboardShortcut::new(
                            egui::Modifiers::COMMAND,
                            egui::Key::K,
                        ));
                        let (rect, response) =
                            ui.allocate_exact_size(vec2(220.0, 24.0), Sense::click());
                        let painter = ui.painter();
                        let hovered = response.hovered();
                        painter.rect(
                            rect,
                            4,
                            if hovered {
                                palette.select
                            } else {
                                palette.head
                            },
                            Stroke::new(1.0, palette.rule),
                            StrokeKind::Inside,
                        );
                        theme::paint_icon(
                            painter,
                            Rect::from_center_size(
                                rect.left_center() + vec2(14.0, 0.0),
                                vec2(14.0, 14.0),
                            ),
                            Icon::Search,
                            palette.muted,
                        );
                        painter.text(
                            rect.left_center() + vec2(28.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            "Find a command",
                            egui::FontId::proportional(11.0),
                            palette.muted,
                        );
                        painter.text(
                            rect.right_center() - vec2(8.0, 0.0),
                            egui::Align2::RIGHT_CENTER,
                            shortcut,
                            egui::FontId::proportional(10.0),
                            palette.muted,
                        );
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::Role::SearchInput,
                                true,
                                "Find a command",
                            )
                        });
                        if response.on_hover_cursor(egui::CursorIcon::Text).clicked() {
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
                // Every viewport command needs a pane to act on.
                let has_pane = self.views.active_pane().is_some();
                let mut pick = None;
                let (mut undo, mut redo) = (false, false);
                let ribbon = egui::ScrollArea::horizontal().id_salt("ribbon-tools").show(ui, |ui| {
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
                                        // Select opens the first group of the Steel and Edit tabs rather than a
                    // group of one button.
                    let select = |ui: &mut Ui, pick: &mut Option<Bench>| {
                        if tool_button(ui, Icon::Select, "Select", matches!(bench, Bench::Select | Bench::View), edits_free)
                            .clicked()
                        {
                            *pick = Some(Bench::Select);
                        }
                    };
                    if self.ribbon_tab == RibbonTab::Steel {
                                        group(ui, "Members", &mut |ui| {
                        select(ui, &mut pick);
                        for (b, icon, label) in [
                            (Bench::Column, Icon::Column, "Column"),
                            (Bench::Beam, Icon::Beam, "Beam"),
                            (Bench::Plate, Icon::Plate, "Plate"),
                        ] {
                            if tool_button(ui, icon, label, bench == b, edits_free).clicked() {
                                pick = Some(b);
                            }
                        }
                        tool_button(ui, Icon::Brace, "Brace", false, false);
                    });
                    group(ui, "Connect", &mut |ui| {
                        if tool_button(ui, Icon::Bolt, "Bolts", bench == Bench::Bolt, true).clicked() {
                            pick = Some(Bench::Bolt);
                        }
                        if tool_button(ui, Icon::Weld, "Weld", bench == Bench::Weld, true).clicked() {
                            pick = Some(Bench::Weld);
                        }
                    });
                    }
                    if self.ribbon_tab == RibbonTab::Edit {
                                        group(ui, "Modify", &mut |ui| {
                        select(ui, &mut pick);
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
                        dropdown_button(ui, Icon::NewView, "Create view", true, |ui| {
                            for (construction, label) in crate::views::Construction::ALL {
                                if ui.button(label).clicked() {
                                    self.begin_view(construction);
                                    ui.close();
                                }
                            }
                        });
                        let shortcut = ui.ctx().format_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::P));
                        let toggled = self.views.active_pane().is_some_and(|p| p.toggled.is_some());
                        if tool_button(ui, Icon::Fit, "3D / plane", toggled, has_pane)
                            .on_hover_text(format!("Look straight at the view plane and back ({shortcut})"))
                            .clicked()
                        {
                            self.toggle_view_plane();
                        }
                                                let mut direction = None;
                        dropdown_button(ui, Icon::ElevationView, "Direction", has_pane, |ui| {
                            for (towards, label) in crate::views::Direction::ALL {
                                if ui.button(label).clicked() {
                                    direction = Some(towards);
                                    ui.close();
                                }
                            }
                        });
                        if let Some(towards) = direction {
                            self.orient_view(towards);
                        }
                    });
                    group(ui, "Windows", &mut |ui| {
                        if tool_button(ui, Icon::Beside, "Open beside", false, has_pane && self.views.panes.len() < crate::views::MAX_WINDOWS).on_hover_text("Open the active view again in a new group on the right").clicked() {
                            self.open_beside(self.views.active());
                        }
                        if tool_button(ui, Icon::CloseOthers, "Close inactive", false, self.views.panes.len() > 1).on_hover_text("Close every view tab but the active one").clicked() {
                            self.close_inactive();
                        }
                        if tool_button(ui, Icon::ViewFrame, "Tab views", false, has_pane).on_hover_text("Every open view as a tab of one group").clicked() {
                            self.arrange(Arrange::Tabs);
                        }
                        if tool_button(ui, Icon::Arrange, "Tile views", false, self.views.panes.len() > 1).on_hover_text("Every open view side by side in a grid").clicked() {
                            self.arrange(Arrange::Tile);
                        }
                        dropdown_button(ui, Icon::Arrange, "Arrange", has_pane, |ui| {
                            for (how, label) in [
                                (Arrange::Tabs, "Tabs"),
                                (Arrange::Tile, "Tile"),
                                (Arrange::TileHorizontal, "Tile horizontal"),
                                (Arrange::TileVertical, "Tile vertical"),
                            ] {
                                if ui.button(label).clicked() {
                                    self.arrange(how);
                                    ui.close();
                                }
                            }
                            ui.separator();
                            if ui.button("Maximize / restore").clicked() {
                                self.arrange(Arrange::ToggleMaximize);
                                ui.close();
                            }
                        });
                    });
                                        group(ui, "Display", &mut |ui| {
                        // The display modes in one dropdown, as a CAD view control shows the
                        // visual style: its icon is the active pane's mode.
                        let modes = [
                            (beamcraft_engine::RenderMode::Wireframe, Icon::Wire, "Wireframe"),
                            (beamcraft_engine::RenderMode::Transparent, Icon::Transparent, "Transparent"),
                            (beamcraft_engine::RenderMode::Grey, Icon::GreyShaded, "Grey"),
                            (beamcraft_engine::RenderMode::Shaded, Icon::Shaded, "Shaded"),
                            (beamcraft_engine::RenderMode::SelectedOnly, Icon::SelectedOnly, "Selected only"),
                        ];
                        let active = self.active_mode();
                        let shown = modes.iter().find(|(m, _, _)| *m == active).map_or(Icon::Shaded, |(_, icon, _)| *icon);
                        let mut chosen = None;
                        dropdown_button(ui, shown, "Visual style", has_pane, |ui| {
                            for (mode, icon, label) in modes {
                                let row = ui.horizontal(|ui| {
                                    theme::icon(ui, icon, 16.0, Palette::current(ui).text);
                                    ui.selectable_label(has_pane && active == mode, label)
                                });
                                if row.inner.clicked() {
                                    chosen = Some(mode);
                                    ui.close();
                                }
                            }
                        });
                        if let Some(mode) = chosen {
                            self.set_active_mode(mode);
                        }
                        if tool_button(ui, Icon::Grid, "Grid guides", self.grids_shown(), has_pane).clicked() {
                            let on = !self.grids_shown();
                            self.set_grids_shown(on);
                            if !on { self.grid_edit = None; }
                        }
                    });
                    group(ui, "Colour", &mut |ui| {
                        if tool_button(ui, Icon::Filter, "Filters", self.views.filter_edit.is_some(), true)
                            .on_hover_text("Make and edit the filters a view shows, colours or hides")
                            .clicked()
                        {
                            let first = self.filter_choices().first().map(|f| f.name.clone());
                            self.open_filter_editor(first);
                        }
                        if tool_button(ui, Icon::Palette, "Colour", self.bench == Bench::Colour, true)
                            .clicked()
                        {
                            pick = Some(Bench::Colour);
                        }
                    });
                    group(ui, "Cut", &mut |ui| {
                        if tool_button(ui, Icon::CutPlane, "Cut plane", self.bench == Bench::CutPlane, true).clicked() {
                            pick = Some(Bench::CutPlane);
                        }
                    });
                    group(ui, "Refresh", &mut |ui| {
                        if tool_button(ui, Icon::Refresh, "Reload", false, true).clicked() {
                            self.reload();
                        }
                        if tool_button(ui, Icon::Refresh, "Redraw views", false, true).clicked() {
                            for pane in &mut self.views.panes {
                                pane.filtered = None;
                            }
                            self.recolour();
                        }
                    });
                    }
                    if matches!(self.ribbon_tab, RibbonTab::Steel | RibbonTab::View) {
                        group(ui, "Inspect", &mut |ui| {
                            tool_button(ui, Icon::Measure, "Measure", false, false);
                            if tool_button(ui, Icon::Fit, "Fit model", false, has_pane).clicked() {
                                self.pending_fit = true;
                            }
                        });
                    }
                    if self.ribbon_tab == RibbonTab::Manage {
                        group(ui, "Grids", &mut |ui| {
                            if tool_button(ui, Icon::Grid, "Rectangular grid", bench == Bench::Grid, true).clicked() {
                                pick = Some(Bench::Grid);
                            }
                            if tool_button(ui, Icon::Table, "Levels", bench == Bench::Levels, true).clicked() {
                                pick = Some(Bench::Levels);
                            }
                            if tool_button(ui, Icon::Callout, "Annotations", bench == Bench::Annotations, true).clicked() {
                                pick = Some(Bench::Annotations);
                            }
                        });
                        group(ui, "Drawings", &mut |ui| {
                            if tool_button(ui, Icon::Sheet, "Sheet editor", self.sheet_editor.open, true).clicked() {
                                self.sheet_editor.open = true;
                            }
                            if tool_button(ui, Icon::Table, "Drawing document", self.drawing_manager.open, true).clicked() {
                                self.drawing_manager.open = true;
                            }
                        });
                                                group(ui, "Workspace", &mut |ui| {
                            if tool_button(ui, Icon::Recent, "Details", self.details, true)
                                .on_hover_text("Revision, navigation and display settings; publish from the header")
                                .clicked()
                            {
                                self.details = true;
                            }
                        });
                    }
                });
                });
                // An overflow hint: a muted chevron at each edge that hides more groups, so a
                // narrow window shows there is more to scroll to (the wheel scrolls too).
                let more_right = ribbon.content_size.x > ribbon.inner_rect.width() + 1.0;
                let more_left = ribbon.state.clamped_offset().x > 1.0;
                for (at, points_right) in [
                    (ribbon.inner_rect.right_center(), true),
                    (ribbon.inner_rect.left_center(), false),
                ] {
                    let show = if points_right { more_right } else { more_left };
                    if !show {
                        continue;
                    }
                    let half = 3.5;
                    let turn = |x: f32, y: f32| {
                        let x = if points_right { x } else { -x };
                        at + egui::vec2(x, y)
                    };
                    ui.painter().line(
                        vec![turn(-half, -5.0), turn(half, 0.0), turn(-half, 5.0)],
                        Stroke::new(1.5, palette.muted),
                    );
                }
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

    /// The inspector: the properties of what is selected, or what the chosen tool needs.
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
                        } else if self.bench == Bench::View {
                            self.views.subject.title()
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
                if self.bench == Bench::View {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                    // The panel keeps its width: a row never grows it.
                    ui.set_max_width(ui.available_width());
                    self.view_panel(ui);
                    return;
                }
                // The tools' action bars stay pinned under their forms, like the properties'.
                if matches!(self.bench, Bench::Bolt | Bench::Weld) {
                    egui::Panel::bottom("tool-actions")
                        .frame(
                            egui::Frame::new()
                                .fill(palette.surface)
                                .inner_margin(egui::Margin::symmetric(0, 4)),
                        )
                        .show(ui, |ui| {
                            if self.bench == Bench::Bolt {
                                self.bolt_actions(ui);
                            } else {
                                self.weld_actions(ui);
                            }
                        });
                }
                // A kind with no editable properties (a bolt group) offers no Apply or Reset:
                // there is nothing to apply (the recheck of issue 562 item 15).
                if self.bench == Bench::Select
                    && self.props.selected().is_some()
                    && self.unsupported_kind_note().is_none()
                {
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
                            Bench::Levels => self.level_panel(ui),
                            Bench::Move | Bench::Copy | Bench::Delete => self.edit_panel(ui),
                            Bench::CutPlane => self.cut_panel(ui),
                            Bench::Colour => self.colour_panel(ui),
                            Bench::Weld => self.weld_panel(ui),
                            Bench::Bolt => self.bolt_panel(ui),
                            Bench::Annotations => self.annotation_panel(ui),
                            Bench::View => self.view_panel(ui),
                        }
                    });
            });
    }

    /// What the active editor last has to tell the user (an outcome or a refusal), for the
    /// status bar: the panel of the bench in use first, then the selection's edits, then the
    /// publication, whose result and refusal must be readable long after the dialog closed
    /// (issue 569 item 16).
    pub(crate) fn status_note(&self) -> Option<Result<&str, &str>> {
        let bench_note = match self.bench {
            Bench::View => self.views.note.as_ref(),
            Bench::Bolt => self.bolt.note.as_ref(),
            Bench::Weld => self.weld.note.as_ref(),
            Bench::Grid => self.grid_note.as_ref(),
            Bench::Levels => self.level_panel.note.as_ref(),
            Bench::Plate | Bench::Beam | Bench::Column => self.tool_note.as_ref(),
            Bench::Annotations => self.annotation.note.as_ref(),
            Bench::Move | Bench::Copy | Bench::Delete => self.edit_note.as_ref(),
            Bench::Select => self.props_note.as_ref().or(self.grid_note.as_ref()),
            Bench::CutPlane => self.cut_note.as_ref(),
            Bench::Colour => None,
        };
        bench_note
            .or(self.edit_note.as_ref())
            .or(self.views.note.as_ref())
            .or(self.publisher.note.as_ref())
            .map(|note| note.as_ref().map(String::as_str).map_err(String::as_str))
    }

    /// The status bar: the selection, snap switches (not available yet) and Details.
    pub(crate) fn status_bar(&mut self, ui: &mut Ui) {
        let palette = Palette::current(ui);
        egui::Panel::bottom("model-status")
            .exact_size(56.0)
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    // No bottom margin: the frame paints outside the content rect, and a
                    // bottom margin ran past the window edge, clipping the Snap button.
                    .inner_margin(egui::Margin {
                        left: 6,
                        right: 6,
                        top: 3,
                        bottom: 0,
                    }),
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
                    // The snap strip takes only the room the entity strip leaves: at a
                    // narrow window it scrolls there instead of being painted over the
                    // filters (issue 562 item 47).
                    let room = (ui.max_rect().right() - ui.min_rect().right()).max(0.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // The slot is one row tall, like the row the switches always hang
                        // from: the scroll area reserves its content's height from the top
                        // of its slot, so a slot as tall as the switches reserved the space
                        // above them and the status row below climbed onto them (the 584
                        // recheck). Its bar stays hidden, since a floating bar covers the
                        // switches it scrolls (the edge fade still marks more to the side),
                        // and the plain wheel scrolls the strip: it has no other direction.
                        // When the strip does not fit, chevrons at both ends say there is
                        // more to the side and scroll it a few switches at a click; a strip
                        // cut at the window edge with no sign of the rest read as missing
                        // snaps (the recheck of issue 562 item 47).
                        let overflow = super::switches::snaps_width() > room;
                        let chevrons = if overflow {
                            2.0 * SNAP_CHEVRON_WIDTH
                        } else {
                            0.0
                        };
                        let extent_key = egui::Id::unique("status-snaps-extent");
                        // The strip's last offset and its largest offset, from the frame before.
                        let (offset, max_offset) = ui
                            .data(|d| d.get_temp::<(f32, f32)>(extent_key))
                            .unwrap_or((0.0, 0.0));
                        let (can_left, can_right) = (offset > 0.5, offset < max_offset - 0.5);
                        // A chevron click scrolls the strip on the next frame, which the
                        // click's repaint brings: the strip is laid out between the two.
                        let step_key = egui::Id::unique("status-snaps-step");
                        let step = ui
                            .data_mut(|d| d.remove_temp::<f32>(step_key))
                            .unwrap_or(0.0);
                        let mut next_step = 0.0;
                        ui.allocate_ui_with_layout(
                            vec2(
                                super::switches::snaps_width().min(room),
                                ui.spacing().interact_size.y,
                            ),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                // Exactly one switch row tall: a framed button grew past the
                                // row and pushed the status row below onto the switches.
                                let chevron = |ui: &mut Ui, text: &str, enabled: bool| {
                                    let sense = if enabled {
                                        egui::Sense::click()
                                    } else {
                                        egui::Sense::hover()
                                    };
                                    let (rect, response) = ui.allocate_exact_size(
                                        vec2(SNAP_CHEVRON_WIDTH, ui.spacing().interact_size.y),
                                        sense,
                                    );
                                    let colour = if !enabled {
                                        palette.muted.gamma_multiply(0.5)
                                    } else if response.hovered() {
                                        palette.text
                                    } else {
                                        palette.muted
                                    };
                                    ui.painter().text(
                                        rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        text,
                                        egui::FontId::proportional(14.0),
                                        colour,
                                    );
                                    enabled && response.clicked()
                                };
                                if overflow && chevron(ui, "‹", can_left) {
                                    next_step = -SNAP_CHEVRON_STEP;
                                }
                                ui.allocate_ui_with_layout(
                                    vec2(
                                        (super::switches::snaps_width().min(room) - chevrons)
                                            .max(0.0),
                                        ui.spacing().interact_size.y,
                                    ),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.style_mut().always_scroll_the_only_direction = true;
                                        let mut area = egui::ScrollArea::horizontal()
                                            .id_salt("status-snaps")
                                            .scroll_bar_visibility(
                                                egui::scroll_area::ScrollBarVisibility::AlwaysHidden,
                                            );
                                        if step != 0.0 {
                                            area = area.horizontal_scroll_offset(
                                                (offset + step).clamp(0.0, max_offset),
                                            );
                                        }
                                        let output = area.show(ui, |ui| {
                                            super::switches::snaps(ui, &mut self.selection);
                                        });
                                        let max = (output.content_size.x
                                            - output.inner_rect.width())
                                        .max(0.0);
                                        let offset = output.state.clamped_offset().x;
                                        ui.data_mut(|d| d.insert_temp(extent_key, (offset, max)));
                                    },
                                );
                                if overflow && chevron(ui, "›", can_right) {
                                    next_step = SNAP_CHEVRON_STEP;
                                }
                            },
                        );
                        if next_step != 0.0 {
                            ui.data_mut(|d| d.insert_temp(step_key, next_step));
                            ui.ctx().request_repaint();
                        }
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
                    // An edit's outcome is told as soon as the command answers, while the
                    // viewport still shows the scene before it until the refresh lands; say
                    // so, rather than a status that runs ahead of the picture.
                    if self.reload_job.is_some() {
                        ui.label(
                            RichText::new("Updating the view…").small().color(palette.muted),
                        );
                    }
                    // What the active editor has to tell, in one place for every panel.
                    if let Some(note) = self.status_note() {
                        ui.separator();
                        match note {
                            Ok(message) => {
                                ui.label(RichText::new(message).size(10.0).color(palette.text));
                            }
                            Err(message) => {
                                ui.label(RichText::new(message).size(10.0).color(palette.error));
                            }
                        }
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
        crate::kit::tool_window("Details", &mut open, ctx)
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
            ("Levels", Bench::Levels),
            ("Beam", Bench::Beam),
            ("Plate", Bench::Plate),
            ("Weld", Bench::Weld),
            ("Bolts", Bench::Bolt),
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
        // The model views (V12): every way to create one, and the view commands.
        for (construction, label) in crate::views::Construction::ALL {
            let label = format!("Create view: {label}");
            if label.to_lowercase().contains(&query)
                && ui
                    .add(egui::Button::new(&label).min_size(vec2(ui.available_width(), 34.0)))
                    .clicked()
            {
                self.begin_view(construction);
                return true;
            }
        }
        for label in [
            "View properties",
            "3D / plane view",
            "Open the view beside",
            "Close inactive views",
            "Tab views",
            "Tile views",
            "Tile views horizontally",
            "Tile views vertically",
            "Maximize or restore the view group",
            "Next open view",
        ] {
            if label.to_lowercase().contains(&query)
                && ui
                    .add(egui::Button::new(label).min_size(vec2(ui.available_width(), 34.0)))
                    .clicked()
            {
                match label {
                    "View properties" => self.show_view_properties(),
                    "3D / plane view" => self.toggle_view_plane(),
                    "Open the view beside" => self.open_beside(self.views.active()),
                    "Close inactive views" => self.close_inactive(),
                    "Tab views" => self.arrange(Arrange::Tabs),
                    "Tile views" => self.arrange(Arrange::Tile),
                    "Tile views horizontally" => self.arrange(Arrange::TileHorizontal),
                    "Tile views vertically" => self.arrange(Arrange::TileVertical),
                    "Maximize or restore the view group" => self.arrange(Arrange::ToggleMaximize),
                    _ => {
                        if let Some(next) = self.views.next_open() {
                            self.activate_view(Some(next));
                        }
                    }
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
            // A running menu create or open must finish in this window: leaving it would lose
            // the model it opens, and an edit would be dropped by the switch that follows.
            let enabled = match label {
                "Undo" | "Redo" => {
                    self.source.workspace.is_some()
                        && !self.other_command_running()
                        && !self.menu_busy()
                }
                "Publish" => {
                    self.publisher.target().is_ok()
                        && !self.other_command_running()
                        && self.publisher.phase() == Phase::Idle
                }
                "Model library" => {
                    self.from_library && !self.other_command_running() && !self.menu_busy()
                }
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

    /// Whether a menu page runs a job that opens a model: a `create-project` on New, or a
    /// remember on Open. The menu waits for it; the name suggestion and the recent-list read are
    /// background reads and do not count.
    pub(crate) fn menu_busy(&self) -> bool {
        self.menu_new.creating() || self.menu_models.opening()
    }

    /// Esc and "‹ Back" in the menu: a sub-page returns to its section, a top-level page closes
    /// the menu and shows the model again. Nothing happens while `menu_busy`.
    pub(crate) fn menu_back(&mut self) {
        if self.menu_busy() {
            return;
        }
        match self.menu_section.parent() {
            Some(parent) => self.menu_section = parent,
            None => self.close_menu(),
        }
    }

    /// Closes the menu. Every way out goes through here (Esc, Cancel, "Back to model"), so page
    /// resets belong here and a reopened menu never shows a stale form or stale facts: the New,
    /// Open and Project pages give way to the default page. Nothing happens while `menu_busy`.
    pub(crate) fn close_menu(&mut self) {
        if self.menu_busy() {
            return;
        }
        if matches!(
            self.menu_section,
            MenuSection::New | MenuSection::Open | MenuSection::Project
        ) {
            self.menu_section = MenuSection::default();
        }
        self.menu_open = false;
        self.menu_note = None;
    }

    /// The menu's New page: the new-model form. A created model becomes the window's
    /// `open_request`; the menu stays open until the window has switched or reported why not.
    fn menu_new_page(&mut self, ui: &mut Ui) {
        let outer_busy = self.other_command_running();
        let host = crate::library::Host {
            cli: &self.source.cli,
            config_dir: self.source.config_dir.as_deref(),
            actor: self.source.actor.as_deref(),
            environment: self.source.environment.as_deref(),
            stats: &self.stats,
        };
        match self.menu_new.ui(ui, &host, outer_busy) {
            Some(crate::library::FormEvent::Created(project)) => {
                self.open_request = Some(std::path::PathBuf::from(project.path));
            }
            Some(crate::library::FormEvent::Cancelled) => self.menu_back(),
            None => {}
        }
    }

    /// The menu's Open page: the recent models and "Browse…". A chosen model becomes the
    /// window's `open_request`, as on the New page.
    fn menu_open_page(&mut self, ui: &mut Ui) {
        let outer_busy = self.other_command_running();
        match self.menu_models.ui(
            ui,
            self.source.config_dir.as_deref(),
            &self.source.store,
            outer_busy,
        ) {
            Some(crate::library::PickEvent::Opened(path)) => self.open_request = Some(path),
            Some(crate::library::PickEvent::Cancelled) => self.menu_back(),
            None => {}
        }
    }

    /// Starts reading the open model's project for the Project page, on a worker thread; the
    /// facts shown before are dropped.
    fn read_project(&mut self) {
        self.project_facts = None;
        let store = self.source.store.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("beamcraft-project".into())
            .spawn(move || {
                let _ = tx.send(crate::library::project_facts(&store));
            });
        if spawned.is_ok() {
            self.project_job = Some(rx);
        } else {
            self.project_job = None;
            self.project_facts = Some(Err("the project could not be read".into()));
        }
    }

    /// Takes the Project page's facts once the read has answered, and repaints while it runs.
    fn poll_project(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.project_job else {
            return;
        };
        match rx.try_recv() {
            Ok(facts) => {
                self.project_job = None;
                self.project_facts = Some(facts);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.project_job = None;
                self.project_facts = Some(Err("the project read ended without an answer".into()));
            }
        }
    }

    /// The menu's Project page: the project as the store's head records it and this window's
    /// session, read-only. Project information fields do not exist in models yet; the page says
    /// so instead of offering fields it cannot keep.
    fn menu_project_page(&mut self, ui: &mut Ui) {
        let palette = Palette::current(ui);
        let key = |ui: &mut Ui, text: &str| {
            ui.label(RichText::new(text).color(palette.muted));
        };
        let time = |ms: Option<u64>| {
            ms.map_or_else(
                || "—".to_owned(),
                |ms| format!("{} UTC", crate::start::format_time(ms)),
            )
        };
        ui.label(theme::bold("Project", 14.0));
        ui.add_space(8.0);
        match &self.project_facts {
            Some(Ok(facts)) => {
                egui::Grid::new("menu-project-facts")
                    .num_columns(2)
                    .spacing(vec2(24.0, 8.0))
                    .show(ui, |ui| {
                        for (name, value) in [
                            ("Name", facts.name.clone()),
                            ("Folder", facts.path.clone()),
                            (
                                "Project ID",
                                facts.project_id.clone().unwrap_or_else(|| "—".to_owned()),
                            ),
                            ("Created", time(facts.created_ms)),
                            ("Last change", time(facts.modified_ms)),
                            ("State", facts.state.clone()),
                        ] {
                            key(ui, name);
                            ui.add(egui::Label::new(value).wrap());
                            ui.end_row();
                        }
                    });
            }
            Some(Err(error)) => {
                ui.label(RichText::new(error).color(palette.error).strong());
            }
            None => {
                ui.spinner();
            }
        }
        ui.add_space(20.0);
        ui.label(theme::bold("This session", 14.0));
        ui.add_space(8.0);
        let revision = self
            .revision_on_screen()
            .unwrap_or_else(|| "No model loaded".to_owned());
        let elements = self
            .loaded
            .as_ref()
            .map_or_else(|| "—".to_owned(), |l| l.summary.elements.to_string());
        let or_dash = |text: &Option<String>| text.clone().unwrap_or_else(|| "—".to_owned());
        egui::Grid::new("menu-project-session")
            .num_columns(2)
            .spacing(vec2(24.0, 8.0))
            .show(ui, |ui| {
                for (name, value) in [
                    ("Environment", or_dash(&self.source.environment)),
                    ("Author", or_dash(&self.source.author)),
                    ("Units", "mm".to_owned()),
                    ("Revision on screen", revision),
                    ("Elements", elements),
                ] {
                    key(ui, name);
                    ui.add(egui::Label::new(value).wrap());
                    ui.end_row();
                }
            });
        ui.add_space(20.0);
        ui.add(
            egui::Label::new(
                RichText::new(
                    "Project information (number, client, building and site address, designer, \
                     dates and custom fields) is not stored in models yet.",
                )
                .color(palette.muted),
            )
            .wrap(),
        );
    }

    /// Full native application menu, using the same real actions as the ribbon.
    pub(crate) fn menu_page(&mut self, ui: &mut Ui) {
        self.poll_project(ui.ctx());
        let shown = self.menu_section;
        self.menu_page_ui(ui);
        // A note belongs to the page it was reported on.
        if self.menu_section != shown {
            self.menu_note = None;
        }
    }

    fn menu_page_ui(&mut self, ui: &mut Ui) {
        let palette = Palette::current(ui);
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.menu_back();
        }
        egui::Panel::left("application-menu-navigation")
            .exact_size(232.0)
            .frame(egui::Frame::new().fill(palette.side).inner_margin(12))
            .show(ui, |ui| {
                // The store-path footer is a bottom panel of this panel: it stays pinned and
                // the navigation scroll takes exactly the rest, so Logs and Help stay reachable
                // at any window height instead of sliding under the footer.
                egui::Panel::bottom("application-menu-footer")
                    .frame(egui::Frame::new())
                    .show(ui, |ui| {
                        ui.separator();
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
                            // The path elides on one line with a tooltip: a deep folder wraps
                            // past the window edge otherwise, and the tail is never drawn
                            // (issue 569 item 26). The elision's own tooltip stays off: it
                            // stacks a second copy of this hover text (issue 584 item 3).
                            egui::Label::new(
                                RichText::new(self.source.store.display().to_string())
                                    .size(10.0)
                                    .color(palette.side_text()),
                            )
                            .truncate()
                            .show_tooltip_when_elided(false),
                        )
                        .on_hover_text(self.source.store.display().to_string());
                    });
                // A short window drops the "Menu" heading, which the title bar repeats, so
                // every entry fits above the footer instead of Help hiding half under it
                // behind a scroll (the recheck of issue 562 item 47).
                let compact = ui.available_height() < MENU_FULL_HEIGHT;
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    // While a page opens a model, the menu waits for it.
                    let idle = !self.menu_busy();
                    if menu_navigation(ui, Icon::Back, "Back to model", false, idle).clicked() {
                        self.close_menu();
                    }
                    if compact {
                        ui.add_space(6.0);
                    } else {
                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            ui.add_space(12.0);
                            ui.label(theme::bold("Menu", 23.0).color(palette.side_text()));
                        });
                        ui.add_space(14.0);
                    }
                    let free = idle && self.from_library && !self.other_command_running();
                    let new = MenuSection::New;
                    if menu_navigation(ui, Icon::Plus, "New", self.menu_section == new, free)
                        .clicked()
                    {
                        self.menu_section = new;
                        if let Some(parent) = self.source.store.parent().and_then(|p| p.to_str()) {
                            self.menu_new.seed_parent(parent);
                        }
                        self.menu_new.show();
                    }
                    let open = MenuSection::Open;
                    if menu_navigation(ui, Icon::Folder, "Open", self.menu_section == open, free)
                        .clicked()
                    {
                        self.menu_section = open;
                        self.menu_models.refresh(self.source.config_dir.as_deref());
                    }
                    menu_navigation(ui, Icon::Copy, "Save as", false, false);
                    if menu_navigation(ui, Icon::Folder, "Open model folder", false, idle).clicked()
                    {
                        self.menu_note = crate::dialog::open_folder(&self.source.store)
                            .err()
                            .map(Err);
                    }
                    for (section, icon, label) in [
                        (MenuSection::Import, Icon::Back, "Import"),
                        (MenuSection::Export, Icon::Shared, "Export"),
                    ] {
                        if menu_navigation(ui, icon, label, self.menu_section == section, idle)
                            .clicked()
                        {
                            self.menu_section = section;
                        }
                    }
                    let project = MenuSection::Project;
                    if menu_navigation(
                        ui,
                        Icon::Block,
                        "Project properties",
                        self.menu_section == project,
                        idle,
                    )
                    .clicked()
                    {
                        self.menu_section = project;
                        self.read_project();
                    }
                    for (section, icon, label) in [
                        (MenuSection::Settings, Icon::Grid, "Settings"),
                        (MenuSection::Catalogs, Icon::Wire, "Catalogs"),
                        (MenuSection::Editors, Icon::Plate, "Editors"),
                    ] {
                        let active = self.menu_section.top() == section;
                        if menu_navigation(ui, icon, label, active, idle).clicked() {
                            self.menu_section = section;
                        }
                    }
                    menu_navigation(ui, Icon::Recent, "Logs", false, false);
                    let help = MenuSection::Help;
                    if menu_navigation(
                        ui,
                        Icon::Recent,
                        "Help",
                        self.menu_section.top() == help,
                        idle,
                    )
                    .clicked()
                    {
                        self.menu_section = help;
                    }
                });
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(palette.page)
                    .inner_margin(egui::Margin::symmetric(40, 36)),
            )
            .show(ui, |ui| {
                let (title, description) = self.menu_section.heading();
                ui.horizontal(|ui| {
                    if self.menu_section.parent().is_some()
                        && theme::button(ui, Kind::Secondary, "‹ Back", !self.menu_busy()).clicked()
                    {
                        self.menu_back();
                    }
                    ui.label(theme::bold(title, 28.0));
                });
                ui.label(RichText::new(description).size(12.0).color(palette.muted));
                match &self.menu_note {
                    Some(Ok(note)) => {
                        ui.add_space(8.0);
                        ui.label(RichText::new(note).color(palette.muted));
                    }
                    Some(Err(error)) => {
                        ui.add_space(8.0);
                        ui.label(RichText::new(error).color(palette.error).strong());
                    }
                    None => {}
                }
                ui.add_space(30.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    // The New page is wider, so the template cards fit side by side.
                    let width = if self.menu_section == MenuSection::New {
                        800.0
                    } else {
                        620.0
                    };
                    ui.set_width(ui.available_width().min(width));
                    ui.separator();
                    ui.add_space(15.0);
                    match self.menu_section {
                        MenuSection::New => {
                            self.menu_new_page(ui);
                            return;
                        }
                        MenuSection::Open => {
                            self.menu_open_page(ui);
                            return;
                        }
                        MenuSection::Project => {
                            self.menu_project_page(ui);
                            return;
                        }
                        _ => {}
                    }
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for label in self.menu_section.commands() {
                        // The IFC row belongs to the Export page only: the Import page lists the
                        // same label and its import command does not exist yet.
                        let enabled = matches!(
                            *label,
                            "Profile catalog" | "Material catalog" | "About BeamCraft"
                        ) || *label == "IFC"
                            && self.menu_section == MenuSection::Export;
                        if menu_command(ui, label, enabled).clicked() {
                            match *label {
                                "Profile catalog" => self.menu_section = MenuSection::Profiles,
                                "Material catalog" => self.menu_section = MenuSection::Materials,
                                "IFC" => self.open_menu_export(),
                                "About BeamCraft" => {
                                    self.menu_section = MenuSection::About;
                                }
                                _ => {}
                            }
                        }
                    }
                    if self.menu_section == MenuSection::Export && self.menu_export_open() {
                        self.menu_export_ui(ui, self.other_command_running());
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
                    .inner_margin(egui::Margin::symmetric(8, 8)),
            )
            .show(ui, |ui| {
                // The Views / Drawings / Schedules tabs, the folders, quick access and the
                // context menu live in the view browser; the 3D entry is the free view.
                self.view_browser(ui);
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
