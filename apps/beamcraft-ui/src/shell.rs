// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The window started without a model: the start page first, then the model chosen on it, in the
//! same window. A model that cannot be opened returns to the start page with the reason. A model
//! window can ask to switch to another model in place; when that one cannot be opened, the
//! current model stays and its menu shows the reason. A window that is replaced or left frees its
//! textures in the next pass, once no output still to be painted refers to them.

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
    /// The window replaced or left in the previous pass: the output of that pass may still paint
    /// its textures, so it is released and dropped at the start of the next pass, whose output
    /// replaces it (egui paints only the last pass of a frame).
    retired: Option<App>,
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
            retired: None,
            cli,
            samples,
            stats,
            error: None,
        }
    }

    /// Replaces the open model with the one `source` names, in the same window. The old window
    /// is retired, not dropped, so its textures are freed only in the next pass. When
    /// the model cannot be opened, the current one stays and its menu shows why.
    fn switch(&mut self, ctx: &egui::Context, frame: &eframe::Frame, source: Source) {
        let title = title_of(&source.store);
        match App::open(
            ctx,
            frame.wgpu_render_state(),
            source,
            None,
            Arc::clone(&self.stats),
        ) {
            Ok(mut next) => {
                next.appearance = self.app.as_mut().and_then(|app| app.appearance.take());
                next.from_library = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
                self.retired = self.app.replace(next);
            }
            Err(e) => {
                if let Some(app) = &mut self.app {
                    app.open_failed(format!("the model could not be opened: {e}"));
                }
            }
        }
    }
}

/// The window title of a model: its folder name, or the application name alone.
fn title_of(store: &Path) -> String {
    store
        .file_name()
        .and_then(|n| n.to_str())
        .map_or_else(|| "BeamCraft".to_owned(), |n| format!("{n} - BeamCraft"))
}

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if let Some(mut old) = self.retired.take() {
            old.release();
        }
        if let Some(app) = &mut self.app {
            app.ui(ui, frame);
            let request = app.take_open_request();
            if let Some(source) = request {
                self.switch(ui.ctx(), frame, source);
            } else if app.return_to_library {
                self.appearance = app.appearance.take();
                self.retired = self.app.take();
                self.start.return_to_models();
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
            // The start page's file picker chose a received IFC file: open it read-only, the
            // same window `beamcraft-ui --source FILE` opens (issue 614).
            if let Some(file) = self.start.take_open_ifc() {
                let title = title_of(&file);
                let source = Source {
                    store: file.clone(),
                    ifc_source: Some(file),
                    cli: self.cli.clone(),
                    workspace: None,
                    published: None,
                    samples: self.samples,
                    actor: None,
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
                    Err(e) => self.error = Some(format!("the file could not be opened: {e}")),
                }
            }
            return;
        };
        let title = title_of(&store);
        let source = Source {
            store,
            ifc_source: None,
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

#[cfg(test)]
mod tests {
    use super::Shell;
    use crate::app::tests::{headless, on_the_menu};
    use std::sync::Arc;

    /// A scratch configuration directory of this test, so no user settings are read or written.
    fn scratch(name: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("bc-shell-{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A Shell on a fresh context with Inter and the theme, as the window has them. Every frame
    /// runs one pass: egui paints only the last pass of a frame, so a window retired in a
    /// discarded pass may be freed in the next pass of the same frame, and the tests count the
    /// delay in single-pass frames.
    fn shell(ctx: &egui::Context, config: &std::path::Path) -> Shell {
        ctx.options_mut(|o| o.max_passes = std::num::NonZeroUsize::MIN);
        crate::theme::install(ctx);
        assert!(crate::app::inter(ctx));
        let missing = std::env::temp_dir().join("beamcraft-window-test-no-cli");
        Shell::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            missing,
            Some(config.to_path_buf()),
            None,
            Arc::new(crate::stats::Stats::default()),
        )
    }

    /// One real `Shell::ui` frame.
    fn run(shell: &mut Shell, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| eframe::App::ui(shell, ui, frame),
        );
        assert_eq!(output.platform_output.num_completed_passes, 1);
        output.drop_without_applying_deltas();
    }

    #[test]
    fn a_failed_switch_keeps_the_current_model() {
        let Some(mut app) = headless() else { return };
        let ctx = egui::Context::default();
        let config = scratch("failed");
        let mut shell = shell(&ctx, &config);
        on_the_menu(&mut app);
        app.open_request = Some(config.join("Other"));
        shell.app = Some(app);
        // No render state: `App::open` cannot build the new window.
        let mut frame = eframe::Frame::_new_kittest();
        run(&mut shell, &ctx, &mut frame);
        let app = shell.app.as_ref().unwrap();
        assert!(shell.retired.is_none());
        assert_eq!(app.open_request, None);
        let Some(Err(note)) = &app.menu_note else {
            panic!("expected an error note, got {:?}", app.menu_note);
        };
        assert!(note.contains("the model could not be opened"), "{note}");
        // The start page worker may still hold the directory; leftovers are only scratch.
        let _ = std::fs::remove_dir_all(&config);
    }

    #[test]
    fn a_switch_retires_the_old_window_and_frees_it_next_frame() {
        let Some(mut app) = headless() else { return };
        let ctx = egui::Context::default();
        let config = scratch("switch");
        let mut shell = shell(&ctx, &config);
        on_the_menu(&mut app);
        app.open_request = Some(config.join("Other"));
        let old = app.texture_id().unwrap();
        let state = app.render_state().clone();
        assert!(app.appearance.is_some());
        shell.app = Some(app);
        let mut frame = eframe::Frame::_new_kittest();
        frame.wgpu_render_state = Some(state.clone());

        run(&mut shell, &ctx, &mut frame);
        let next = shell.app.as_ref().unwrap();
        assert!(next.from_library);
        assert!(next.appearance.is_some());
        assert_ne!(next.texture_id(), Some(old));
        let retired = shell.retired.as_ref().unwrap();
        assert!(retired.appearance.is_none());
        // The frame just built may still paint the old texture: it is freed only next frame.
        assert!(state.renderer.read().texture(&old).is_some());

        run(&mut shell, &ctx, &mut frame);
        assert!(shell.retired.is_none());
        assert!(state.renderer.read().texture(&old).is_none());
        // The start page worker may still hold the directory; leftovers are only scratch.
        let _ = std::fs::remove_dir_all(&config);
    }
}
