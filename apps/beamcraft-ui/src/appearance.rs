// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Appearance settings shared by the start and model windows, kept in the user's settings file
//! through the `session` crate on a worker thread.

use crate::theme::Palette;
use crate::tools::ToolError;
use egui::ThemePreference;
use session::settings::{self, Settings};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

/// The egui appearance for a stored preference.
pub const fn egui_theme(theme: settings::ThemePreference) -> ThemePreference {
    match theme {
        settings::ThemePreference::System => ThemePreference::System,
        settings::ThemePreference::Light => ThemePreference::Light,
        settings::ThemePreference::Dark => ThemePreference::Dark,
    }
}

/// The stored preference for an egui appearance.
pub const fn stored_theme(theme: ThemePreference) -> settings::ThemePreference {
    match theme {
        ThemePreference::System => settings::ThemePreference::System,
        ThemePreference::Light => settings::ThemePreference::Light,
        ThemePreference::Dark => settings::ThemePreference::Dark,
    }
}

/// Reads or writes the stored preference of the settings in `config_dir` (the current user's
/// directory when `None`), under the settings write lock of this process.
fn stored(
    config_dir: Option<&PathBuf>,
    change: Option<ThemePreference>,
) -> Result<ThemePreference, ToolError> {
    let dir = config_dir
        .cloned()
        .or_else(settings::default_dir)
        .ok_or_else(|| ToolError::local("NOT_FOUND", "no configuration directory is known"))?;
    let failed = |e: settings::SettingsError| ToolError::local(e.code.as_str(), e.message);
    let settings = Settings::update(&dir, |settings| match change {
        Some(theme) => {
            settings.theme = stored_theme(theme);
            Ok(true)
        }
        None => Ok(false),
    })
    .map_err(failed)?;
    Ok(egui_theme(settings.theme))
}

/// A persistent appearance selector over the user's settings file.
pub struct Appearance {
    config_dir: Option<PathBuf>,
    preference: ThemePreference,
    job: Option<mpsc::Receiver<Result<ThemePreference, ToolError>>>,
    error: Option<String>,
    retry: Option<ThemePreference>,
}

impl Appearance {
    /// Loads the preference asynchronously, using the same configuration as the start page.
    pub fn new(config_dir: Option<PathBuf>) -> Self {
        let mut this = Self {
            config_dir,
            preference: ThemePreference::System,
            job: None,
            error: None,
            retry: None,
        };
        this.request(None);
        this
    }

    fn request(&mut self, preference: Option<ThemePreference>) {
        self.retry = preference;
        let config_dir = self.config_dir.clone();
        let (tx, rx) = mpsc::channel();
        match std::thread::Builder::new()
            .name("appearance-settings".into())
            .spawn(move || {
                let _ = tx.send(stored(config_dir.as_ref(), preference));
            }) {
            Ok(_) => {
                self.job = Some(rx);
                self.error = None;
            }
            Err(error) => self.error = Some(format!("Appearance worker could not start: {error}")),
        }
    }

    /// Applies a completed settings request without blocking the window.
    pub fn poll(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.job {
            match rx.try_recv() {
                Ok(Ok(preference)) => {
                    self.preference = preference;
                    if !matches!(
                        std::env::var("BEAMCRAFT_THEME").as_deref(),
                        Ok("light" | "dark")
                    ) {
                        ctx.set_theme(preference);
                        ctx.request_discard("Apply appearance to the entire frame");
                    }
                    self.job = None;
                }
                Ok(Err(error)) => {
                    self.error = Some(error.to_string());
                    self.job = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.error = Some("Appearance worker ended without an answer".into());
                    self.job = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    /// Draws the welcome title bar, including its initial appearance choice.
    pub fn ui(&mut self, ui: &mut egui::Ui, enabled: bool) {
        self.titlebar(ui, enabled, false);
    }

    /// Start-screen title bar with optional maximize for a resizable library.
    pub fn titlebar(&mut self, ui: &mut egui::Ui, enabled: bool, maximize: bool) {
        self.poll(ui.ctx());
        let palette = Palette::current(ui);
        egui::Panel::top("application-appearance")
            .exact_size(36.0)
            .frame(
                egui::Frame::new()
                    .fill(palette.head)
                    .inner_margin(egui::Margin::symmetric(14, 0)),
            )
            .show(ui, |ui| {
                crate::theme::titlebar_drag_with_maximize(ui, maximize);
                ui.horizontal_centered(|ui| {
                    ui.label(egui::RichText::new("BeamCraft").size(12.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        crate::theme::window_controls_with_maximize(ui, maximize);
                        if !maximize {
                            self.selector(ui, enabled);
                        } else {
                            self.error_ui(ui, enabled);
                        }
                    });
                });
            });
    }

    /// Draws the initial appearance choice on the welcome screen.
    pub fn selector(&mut self, ui: &mut egui::Ui, enabled: bool) {
        let palette = Palette::current(ui);
        let override_theme = match std::env::var("BEAMCRAFT_THEME").as_deref() {
            Ok("light") => Some(ThemePreference::Light),
            Ok("dark") => Some(ThemePreference::Dark),
            _ => None,
        };
        let mut selected = override_theme.unwrap_or(self.preference);
        ui.add_enabled_ui(
            enabled && self.job.is_none() && override_theme.is_none(),
            |ui| {
                egui::ComboBox::from_id_salt("appearance-preference")
                    .width(82.0)
                    .selected_text(match selected {
                        ThemePreference::System => "System",
                        ThemePreference::Light => "Light",
                        ThemePreference::Dark => "Dark",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut selected, ThemePreference::System, "System");
                        ui.selectable_value(&mut selected, ThemePreference::Light, "Light");
                        ui.selectable_value(&mut selected, ThemePreference::Dark, "Dark");
                    });
            },
        )
        .response
        .on_disabled_hover_text("Loading/saving settings, or BEAMCRAFT_THEME overrides appearance");
        ui.label(egui::RichText::new("Theme").size(11.0).color(palette.muted));
        if override_theme.is_none() && selected != self.preference {
            self.request(Some(selected));
        }
        self.error_ui(ui, enabled);
    }

    /// Reports appearance failures even on screens without an appearance selector.
    pub fn error_ui(&mut self, ui: &mut egui::Ui, enabled: bool) {
        let palette = Palette::current(ui);
        if let Some(error) = self.error.clone() {
            if ui
                .add_enabled(enabled, egui::Button::new("Retry appearance"))
                .clicked()
            {
                self.request(self.retry);
            }
            ui.colored_label(palette.error, error);
        }
    }

    /// Whether a settings read/write is in flight.
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_preferences_round_trip_between_egui_and_the_settings() {
        for theme in [
            ThemePreference::System,
            ThemePreference::Light,
            ThemePreference::Dark,
        ] {
            assert_eq!(egui_theme(stored_theme(theme)), theme);
        }
    }

    #[test]
    fn a_saved_preference_is_read_back_and_a_read_changes_nothing() {
        let dir = std::env::temp_dir().join(format!("ui-appearance-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let config = Some(dir.clone());
        assert_eq!(stored(config.as_ref(), None), Ok(ThemePreference::System));
        assert!(!dir.join("settings.json").exists(), "a read writes nothing");
        assert_eq!(
            stored(config.as_ref(), Some(ThemePreference::Dark)),
            Ok(ThemePreference::Dark)
        );
        assert_eq!(stored(config.as_ref(), None), Ok(ThemePreference::Dark));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
