// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The original BeamCraft workspace drawer. Its command entries are presentation-only
//! placeholders: disabled widgets dispatch no commands and change no model data.

use crate::theme::{self, Palette};
use egui::{Ui, vec2};

const GROUPS: &[(&str, &[&str])] = &[
    (
        "MODEL FILES",
        &["New model", "Open model", "Duplicate model", "Model folder"],
    ),
    (
        "EXCHANGE & OUTPUT",
        &["Import reference", "Export model", "Print drawings"],
    ),
    (
        "PROJECT",
        &["Project information", "Libraries", "Preferences"],
    ),
    (
        "SUPPORT",
        &["Activity log", "Check model", "Help", "About BeamCraft"],
    ),
    ("SESSION", &["Close model", "Exit BeamCraft"]),
];

fn entry(ui: &mut Ui, label: &str) -> egui::Response {
    ui.add_enabled(
        false,
        egui::Button::new("")
            .left_text(label)
            .frame(false)
            .min_size(vec2(ui.available_width(), 36.0)),
    )
    .on_disabled_hover_text("Not available yet")
}

/// Slides a docked panel into the model layout; the rest of the window stays interactive.
pub(super) fn show(ui: &mut Ui, open: &mut bool) {
    if *open && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        *open = false;
    }
    let palette = Palette::current(ui);
    let mut close = false;
    egui::Panel::left("workspace-menu")
        .exact_size(264.0)
        .resizable(false)
        .frame(egui::Frame::new().fill(palette.side).inner_margin(16))
        .show_collapsible(ui, open, |ui| {
            ui.visuals_mut().override_text_color = Some(palette.side_text());
            ui.horizontal(|ui| {
                ui.label(theme::bold("Workspace", 22.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new("×")
                                .frame(false)
                                .min_size(vec2(44.0, 44.0)),
                        )
                        .on_hover_text("Close menu (Esc)")
                        .clicked()
                    {
                        close = true;
                    }
                });
            });
            ui.separator();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (heading, entries) in GROUPS {
                        ui.add_space(12.0);
                        ui.label(theme::bold(*heading, 11.0));
                        for label in *entries {
                            entry(ui, label);
                        }
                    }
                });
        });
    if close {
        *open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_dismisses_the_drawer_and_entries_are_disabled() {
        let ctx = egui::Context::default();
        crate::app::inter(&ctx);
        theme::install(&ctx);
        let mut open = true;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                vec2(900.0, 700.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input.clone(), |ui| {
            show(ui, &mut open);
            egui::CentralPanel::default().show(ui, |ui| {
                for (_, entries) in GROUPS {
                    for label in *entries {
                        let response = entry(ui, label);
                        assert!(!response.enabled());
                        assert!(!response.clicked());
                    }
                }
            });
        });
        // This input test has no painter; deliberately discard texture uploads.
        output.textures_delta.clear();
        assert!(open);
        let mut input = input;
        input.events.push(egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        let mut output = ctx.run_ui(input, |ui| show(ui, &mut open));
        output.textures_delta.clear();
        assert!(!open);
    }
}
