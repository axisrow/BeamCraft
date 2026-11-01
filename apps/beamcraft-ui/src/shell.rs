// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The window started without a model: the start page first, then the model chosen on it, in the
//! same window. A model that cannot be opened returns to the start page with the reason.

use crate::app::{App, Source};
use crate::start::StartPage;
use crate::stats::Stats;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The start page, then the window of the opened model.
pub struct Shell {
    appearance: Option<crate::appearance::Appearance>,
    start: StartPage,
    app: Option<App>,
    cli: PathBuf,
    samples: Option<u32>,
    stats: Arc<Stats>,
    error: Option<String>,
}

impl Shell {
    /// A window that starts on the start page; `config_dir` replaces the user's configuration
    /// directory for every start command.
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        cli: PathBuf,
        config_dir: Option<PathBuf>,
        samples: Option<u32>,
        stats: Arc<Stats>,
    ) -> Self {
        crate::app::inter(&cc.egui_ctx);
        crate::theme::install(&cc.egui_ctx);
        Self {
            appearance: Some(crate::appearance::Appearance::new(config_dir.clone())),
            start: StartPage::new(cli.clone(), config_dir, Arc::clone(&stats)),
            app: None,
            cli,
            samples,
            stats,
            error: None,
        }
    }
}

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if let Some(app) = &mut self.app {
            app.ui(ui, frame);
            if app.return_to_library {
                let create = app.return_to_create;
                self.appearance = app.appearance.take();
                self.app = None;
                self.start.return_to_models();
                if create {
                    self.start.show_create();
                }
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Title("BeamCraft".into()));
            }
            return;
        }
        if let Some(appearance) = &mut self.appearance {
            appearance.titlebar(ui, !self.start.busy(), self.start.resizable());
        }
        if let Some(error) = &self.error {
            egui::Panel::bottom("start-error").show(ui, |ui| {
                ui.colored_label(egui::Color32::from_rgb(230, 80, 60), error);
            });
        }
        if self
            .appearance
            .as_ref()
            .is_some_and(crate::appearance::Appearance::busy)
        {
            ui.disable();
        }
        let Some(store) = self.start.ui(ui) else {
            return;
        };
        let title = store
            .file_name()
            .and_then(|n| n.to_str())
            .map_or_else(|| "BeamCraft".to_owned(), |n| format!("{n} - BeamCraft"));
        let source = Source {
            store,
            cli: self.cli.clone(),
            workspace: None,
            published: None,
            samples: self.samples,
            actor: self.start.actor_id(),
            environment: self.start.environment().map(str::to_owned),
            author: self.start.author_name(),
            config_dir: self.start.config_dir().map(Path::to_path_buf),
        };
        match App::open(
            ui.ctx(),
            frame.wgpu_render_state(),
            source,
            None,
            Arc::clone(&self.stats),
        ) {
            Ok(mut app) => {
                app.appearance = self.appearance.take();
                app.from_library = true;
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Title(title));
                self.error = None;
                self.app = Some(app);
            }
            Err(e) => self.error = Some(format!("the model could not be opened: {e}")),
        }
    }
}
