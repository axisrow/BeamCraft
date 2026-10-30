// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Compact engine-backed entity and snap strips, with disabled placeholders for future tools.

use crate::selection::Selection;
use crate::theme::{self, Icon, Palette};
use beamcraft_engine::query::pick::{SelectionClass, SnapKind};
use egui::{Color32, Painter, Rect, Response, Stroke, StrokeKind, Ui, pos2, vec2};

const SIZE: f32 = 24.0;
const GAP: f32 = 2.0;

#[derive(Clone, Copy)]
enum Glyph {
    Existing(Icon),
    Component,
    Points,
    Cut,
    Surface,
    Handle,
    Edge,
    Face,
    Vertex,
    Center,
    Intersection,
    Perpendicular,
    Extension,
    Tangent,
    Parallel,
    Ortho,
    ReferencePoint,
    GeometryPoint,
    OnLine,
}

const ENTITIES: &[(&str, Glyph)] = &[
    ("Select objects", Glyph::Existing(Icon::Select)),
    ("Select components", Glyph::Component),
    ("Select parts", Glyph::Existing(Icon::Plate)),
    ("Select bolts", Glyph::Existing(Icon::Bolt)),
    ("Select welds", Glyph::Existing(Icon::Weld)),
    ("Select construction lines", Glyph::Edge),
    ("Select assemblies", Glyph::Existing(Icon::Block)),
    ("Select grids", Glyph::Existing(Icon::Grid)),
    ("Select construction points", Glyph::Points),
    ("Select cuts", Glyph::Cut),
    ("Select fittings", Glyph::Perpendicular),
    ("Select surfaces", Glyph::Surface),
    ("Select reinforcement", Glyph::Parallel),
    ("Select reference models", Glyph::Existing(Icon::Wire)),
    ("Select handles", Glyph::Handle),
    ("Select edges", Glyph::Extension),
    ("Select faces", Glyph::Face),
    ("Select vertices", Glyph::Vertex),
    ("Select holes", Glyph::Center),
    ("Select items", Glyph::Component),
];

const SNAPS: &[(&str, Glyph)] = &[
    ("Snap grid intersections", Glyph::Intersection),
    ("Snap endpoints", Glyph::Existing(Icon::End)),
    ("Snap axis centers", Glyph::Center),
    ("Snap midpoints", Glyph::Existing(Icon::Middle)),
    ("Snap nearest", Glyph::OnLine),
    ("Snap perpendicular", Glyph::Perpendicular),
    ("Snap extensions", Glyph::Extension),
    ("Snap tangents", Glyph::Tangent),
    ("Snap parallel", Glyph::Parallel),
    ("Snap grids", Glyph::Existing(Icon::Grid)),
    ("Snap construction objects", Glyph::ReferencePoint),
    ("Orthogonal constraint", Glyph::Ortho),
    ("Snap reference geometry", Glyph::ReferencePoint),
    ("Snap physical geometry", Glyph::GeometryPoint),
];

fn paint(painter: &Painter, rect: Rect, glyph: Glyph, color: Color32, accent: Color32) {
    if let Glyph::Existing(icon) = glyph {
        theme::paint_icon(painter, rect, icon, color);
        return;
    }
    let at = |x: f32, y: f32| {
        pos2(
            rect.left() + x * rect.width() / 20.0,
            rect.top() + y * rect.height() / 20.0,
        )
    };
    let stroke = Stroke::new(1.25, color);
    let segment = |a: [f32; 2], b: [f32; 2]| {
        let [x, y] = a;
        let [u, v] = b;
        painter.line_segment([at(x, y), at(u, v)], stroke);
    };
    let box_at = |x: f32, y: f32, size: f32| {
        painter.rect_stroke(
            Rect::from_center_size(at(x, y), vec2(size, size)),
            0,
            stroke,
            StrokeKind::Inside,
        );
    };
    let dot = |x: f32, y: f32| {
        painter.circle_filled(at(x, y), 1.8, accent);
    };
    match glyph {
        Glyph::Component => {
            for (x, y) in [(5.0, 5.0), (15.0, 5.0), (10.0, 15.0)] {
                box_at(x, y, 5.0);
            }
            segment([5.0, 8.0], [10.0, 12.0]);
            segment([15.0, 8.0], [10.0, 12.0]);
        }
        Glyph::Points => {
            for (x, y) in [
                (4.0, 4.0),
                (16.0, 4.0),
                (10.0, 10.0),
                (4.0, 16.0),
                (16.0, 16.0),
            ] {
                dot(x, y);
            }
        }
        Glyph::Cut => {
            box_at(10.0, 10.0, 12.0);
            segment([2.0, 17.0], [17.0, 2.0]);
            dot(10.0, 10.0);
        }
        Glyph::Surface | Glyph::Face => {
            painter.add(egui::Shape::closed_line(
                [[3.0, 6.0], [14.0, 3.0], [18.0, 14.0], [7.0, 17.0]]
                    .map(|[x, y]| at(x, y))
                    .to_vec(),
                stroke,
            ));
            if matches!(glyph, Glyph::Surface) {
                segment([7.0, 5.0], [11.0, 16.0]);
                segment([11.0, 4.0], [15.0, 15.0]);
            } else {
                dot(10.0, 10.0);
            }
        }
        Glyph::Handle => {
            segment([4.0, 15.0], [16.0, 5.0]);
            box_at(4.0, 15.0, 5.0);
            box_at(16.0, 5.0, 5.0);
        }
        Glyph::Edge => {
            segment([3.0, 16.0], [17.0, 4.0]);
            dot(3.0, 16.0);
            dot(17.0, 4.0);
        }
        Glyph::Vertex => {
            segment([4.0, 15.0], [10.0, 5.0]);
            segment([10.0, 5.0], [17.0, 15.0]);
            dot(10.0, 5.0);
        }
        Glyph::Center => {
            painter.circle_stroke(at(10.0, 10.0), 6.5, stroke);
            dot(10.0, 10.0);
        }
        Glyph::OnLine => {
            segment([3.0, 4.0], [17.0, 4.0]);
            segment([3.0, 4.0], [17.0, 16.0]);
            segment([17.0, 4.0], [3.0, 16.0]);
            segment([3.0, 16.0], [17.0, 16.0]);
            dot(10.0, 10.0);
        }
        Glyph::Intersection => {
            segment([3.0, 5.0], [17.0, 15.0]);
            segment([5.0, 17.0], [15.0, 3.0]);
            dot(10.0, 10.0);
        }
        Glyph::Perpendicular | Glyph::Ortho => {
            segment([3.0, 16.0], [17.0, 16.0]);
            segment([8.0, 16.0], [8.0, 3.0]);
            segment([8.0, 11.0], [13.0, 11.0]);
            segment([13.0, 11.0], [13.0, 16.0]);
            if matches!(glyph, Glyph::Ortho) {
                dot(8.0, 16.0);
            }
        }
        Glyph::Extension => {
            segment([2.0, 15.0], [10.0, 9.0]);
            for (a, b) in [([12.0, 7.5], [14.0, 6.0]), ([16.0, 4.5], [18.0, 3.0])] {
                segment(a, b);
            }
            dot(10.0, 9.0);
        }
        Glyph::Tangent => {
            painter.circle_stroke(at(8.0, 12.0), 5.0, stroke);
            segment([3.0, 3.0], [18.0, 12.0]);
            dot(11.0, 7.0);
        }
        Glyph::Parallel => {
            segment([2.0, 13.0], [14.0, 3.0]);
            segment([6.0, 17.0], [18.0, 7.0]);
            segment([7.0, 7.0], [9.0, 10.0]);
            segment([11.0, 11.0], [13.0, 14.0]);
        }
        Glyph::ReferencePoint | Glyph::GeometryPoint => {
            if matches!(glyph, Glyph::ReferencePoint) {
                box_at(10.0, 10.0, 12.0);
                segment([2.0, 18.0], [18.0, 2.0]);
            } else {
                box_at(10.0, 10.0, 12.0);
                dot(4.0, 4.0);
            }
            dot(10.0, 10.0);
        }
        Glyph::Existing(_) => {}
    }
}

fn button(ui: &mut Ui, label: &str, glyph: Glyph, active: bool, enabled: bool) -> Response {
    let palette = Palette::current(ui);
    let response = ui
        .scope(|ui| {
            ui.spacing_mut().interact_size = vec2(SIZE, SIZE);
            ui.spacing_mut().button_padding = egui::Vec2::ZERO;
            ui.add_enabled(
                enabled,
                egui::Button::new("")
                    .min_size(vec2(SIZE, SIZE))
                    .fill(if active { palette.select } else { palette.head })
                    .stroke(Stroke::new(
                        1.0,
                        if active { theme::BRAND } else { palette.rule },
                    ))
                    .corner_radius(1),
            )
        })
        .inner;
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    let ink = if enabled { palette.text } else { palette.muted };
    paint(
        ui.painter(),
        response.rect.shrink(4.0),
        glyph,
        ink,
        if enabled { theme::BRAND } else { palette.muted },
    );
    response
        .on_hover_text(label)
        .on_disabled_hover_text(format!("{label} — not available yet (UI only)"))
}

/// Cursor feedback uses the same original symbol vocabulary as the toolbar.
pub(super) fn snap_marker(painter: &Painter, rect: Rect, kind: SnapKind, color: Color32) {
    let glyph = match kind {
        SnapKind::Midpoint => Glyph::Existing(Icon::Middle),
        SnapKind::Center => Glyph::Center,
        SnapKind::GridIntersection => Glyph::Intersection,
        SnapKind::Axis | SnapKind::Edge | SnapKind::GridLine => Glyph::OnLine,
        _ => Glyph::Existing(Icon::End),
    };
    paint(painter, rect, glyph, color, color);
}

fn separator(ui: &mut Ui) {
    let palette = Palette::current(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(5.0, SIZE), egui::Sense::hover());
    ui.painter().line_segment(
        [
            rect.center_top() + vec2(0.0, 3.0),
            rect.center_bottom() - vec2(0.0, 3.0),
        ],
        Stroke::new(1.0, palette.rule),
    );
}

fn entity_class(label: &str) -> Option<SelectionClass> {
    match label {
        "Select parts" => Some(SelectionClass::Parts),
        "Select assemblies" => Some(SelectionClass::Assemblies),
        "Select bolts" => Some(SelectionClass::Bolts),
        "Select welds" => Some(SelectionClass::Welds),
        "Select holes" => Some(SelectionClass::Holes),
        "Select grids" => Some(SelectionClass::Grids),
        "Select construction lines" => Some(SelectionClass::ConstructionObjects),
        "Select items" => Some(SelectionClass::Items),
        _ => None,
    }
}

fn snap_kinds(label: &str) -> &'static [SnapKind] {
    match label {
        "Snap grid intersections" => &[SnapKind::GridIntersection],
        "Snap endpoints" => &[SnapKind::Vertex, SnapKind::AxisEnd, SnapKind::GridEnd],
        "Snap axis centers" => &[SnapKind::Center],
        "Snap midpoints" => &[SnapKind::Midpoint],
        "Snap nearest" => &[SnapKind::Axis, SnapKind::Edge, SnapKind::GridLine],
        "Snap construction objects" => &[
            SnapKind::ConstructionPoint,
            SnapKind::ConstructionEnd,
            SnapKind::ConstructionLine,
            SnapKind::ConstructionCenter,
            SnapKind::ConstructionCircle,
            SnapKind::ConstructionPlane,
        ],
        _ => &[],
    }
}

/// The real select action, engine filters and reserved entity controls in a single dense row.
pub(super) fn entities(ui: &mut Ui, selecting: bool, selection: &mut Selection) -> bool {
    let mut select = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(GAP, 0.0);
        for (index, (label, glyph)) in ENTITIES.iter().enumerate() {
            if matches!(index, 6 | 14) {
                separator(ui);
            }
            let class = entity_class(label);
            let active = class.map_or(index == 0 && selecting, |class| selection.is_on(class));
            let response = button(ui, label, *glyph, active, index == 0 || class.is_some());
            if response.clicked()
                && let Some(class) = class
            {
                selection.set(class, !active);
            }
            if index == 0 && response.clicked() {
                select = true;
            }
        }
    });
    select
}

/// Engine-backed snap switches with disabled placeholders for unsupported constraints.
pub(super) fn snaps(ui: &mut Ui, selection: &mut Selection) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(GAP, 0.0);
        for (index, (label, glyph)) in SNAPS.iter().enumerate() {
            if matches!(index, 11 | 12) {
                separator(ui);
            }
            let kinds = snap_kinds(label);
            let source = matches!(
                *label,
                "Snap grids"
                    | "Snap construction objects"
                    | "Snap reference geometry"
                    | "Snap physical geometry"
                    | "Snap nearest"
                    | "Orthogonal constraint"
            );
            let active = match *label {
                "Snap grids" => selection.snap_grids,
                "Snap construction objects" => selection.snap_construction,
                "Snap reference geometry" => selection.snap_reference,
                "Snap physical geometry" => selection.snap_geometry,
                "Snap nearest" => selection.snap_nearest,
                "Orthogonal constraint" => selection.orthogonal,
                _ => !kinds.is_empty() && kinds.iter().all(|kind| selection.snap_on(*kind)),
            };
            let clicked = button(ui, label, *glyph, active, source || !kinds.is_empty()).clicked();
            if clicked {
                match *label {
                    "Snap grids" => selection.snap_grids = !active,
                    "Snap construction objects" => selection.snap_construction = !active,
                    "Snap reference geometry" => selection.snap_reference = !active,
                    "Snap physical geometry" => selection.snap_geometry = !active,
                    "Snap nearest" => selection.snap_nearest = !active,
                    "Orthogonal constraint" => selection.orthogonal = !active,
                    _ => {
                        for kind in kinds {
                            selection.set_snap(*kind, !active);
                        }
                    }
                }
            }
        }
    });
}

pub(super) fn snaps_width() -> f32 {
    SNAPS.len() as f32 * (SIZE + GAP) - GAP + 2.0 * (5.0 + GAP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_are_compact_complete_and_unimplemented_switches_cannot_be_clicked() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut labels = std::collections::BTreeSet::new();
        assert_eq!(ENTITIES.len(), 20);
        assert_eq!(SNAPS.len(), 14);
        for (label, _) in ENTITIES.iter().chain(SNAPS) {
            assert!(labels.insert(label));
        }
        for class in SelectionClass::ALL {
            assert_eq!(
                ENTITIES
                    .iter()
                    .filter(|(label, _)| entity_class(label) == Some(class))
                    .count(),
                1
            );
        }
        for kind in SnapKind::ALL {
            assert_eq!(
                SNAPS
                    .iter()
                    .filter(|(label, _)| snap_kinds(label).contains(&kind))
                    .count(),
                1
            );
        }
        for preference in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
            ctx.set_theme(preference);
            let mut bounds = Vec::new();
            let output = ctx.run_ui(Default::default(), |ui| {
                ui.spacing_mut().item_spacing = vec2(GAP, 0.0);
                ui.horizontal(|ui| {
                    for (index, (label, glyph)) in ENTITIES.iter().enumerate() {
                        let response = button(ui, label, *glyph, index == 0, index == 0);
                        assert_eq!(response.enabled(), index == 0);
                        assert!(!response.clicked());
                        bounds.push(response.rect);
                    }
                });
                ui.horizontal(|ui| {
                    for (label, glyph) in SNAPS {
                        let response = button(ui, label, *glyph, false, false);
                        assert!(!response.enabled() && !response.clicked());
                        bounds.push(response.rect);
                    }
                });
            });
            output.drop_without_applying_deltas();
            assert!(bounds.iter().all(|rect| rect.size() == vec2(SIZE, SIZE)));
            assert!(bounds.windows(2).all(|pair| {
                let [a, b] = pair else { return false };
                a.top() != b.top() || a.right() <= b.left()
            }));
            // Fourteen switches since T02 added the construction-objects source; the strip
            // grows by one button and stays a single row.
            assert!(snaps_width() < 380.0);
        }
    }
}
