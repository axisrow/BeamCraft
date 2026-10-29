// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Appearance settings shared by the start and model windows, through the headless CLI.

use crate::command;
use crate::start::parse_settings;
use crate::stats::Stats;
use crate::theme::Palette;
use crate::tools::ToolError;
use egui::ThemePreference;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::time::Duration;

/// The stable CLI spelling of an appearance preference.
pub const fn spelling(theme: ThemePreference) -> &'static str {
    match theme {
        ThemePreference::System => "system",
        ThemePreference::Light => "light",
        ThemePreference::Dark => "dark",
    }
}

/// Reads the settings spelling without silently accepting an unknown preference.
pub fn parse(value: &str) -> Result<ThemePreference, ToolError> {
    match value {
        "system" => Ok(ThemePreference::System),
        "light" => Ok(ThemePreference::Light),
        "dark" => Ok(ThemePreference::Dark),
        _ => Err(ToolError::local(
            "UNREADABLE_REPLY",
            "unknown appearance preference",
        )),
    }
}

/// A persistent appearance selector; no user settings file is accessed by the UI.
pub struct Appearance {
    cli: PathBuf,
    config_dir: Option<PathBuf>,
    stats: Arc<Stats>,
    preference: ThemePreference,
    job: Option<mpsc::Receiver<Result<ThemePreference, ToolError>>>,
    error: Option<String>,
    retry: Option<ThemePreference>,
}

impl Appearance {
    /// Loads the preference asynchronously, using the same configuration as the start page.
    pub fn new(cli: PathBuf, config_dir: Option<PathBuf>, stats: Arc<Stats>) -> Self {
        let mut this = Self {
            cli,
            config_dir,
            stats,
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
        let cli = self.cli.clone();
        let stats = Arc::clone(&self.stats);
        let mut args = match preference {
            Some(theme) => vec![
                "set-settings".into(),
                "--theme".into(),
                spelling(theme).into(),
            ],
            None => vec!["show-settings".into()],
        };
        if let Some(dir) = &self.config_dir {
            args.extend(["--config-dir".into(), dir.to_string_lossy().into_owned()]);
        }
        let (tx, rx) = mpsc::channel();
        match std::thread::Builder::new()
            .name("appearance-settings".into())
            .spawn(move || {
                let result = command::run(&cli, &args, Duration::from_secs(60), &stats)
                    .and_then(|out| parse_settings(out.code, &out.stdout))
                    .map(|settings| settings.theme);
                let _ = tx.send(result);
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
    fn all_preferences_round_trip_and_unknown_values_fail() {
        for theme in [
            ThemePreference::System,
            ThemePreference::Light,
            ThemePreference::Dark,
        ] {
            assert_eq!(parse(spelling(theme)), Ok(theme));
        }
        assert!(parse("automatic").is_err());
    }
}
