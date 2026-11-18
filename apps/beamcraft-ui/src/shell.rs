// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The window started without a model: the start page first, then the model chosen on it, in the
//! same window. A model that cannot be opened returns to the start page with the reason. A model
//! window can ask to switch to another model in place; when that one cannot be opened, the
//! current model stays and its menu shows the reason. A window that is replaced or left frees its
//! textures in the next pass, once no output still to be painted refers to them.
//!
//! Before a model is left (the window closed, another model opened, or back to the library) the
//! window takes a picture of its active view and saves it as the model's `thumbnail.png`
//! ([`crate::thumbnail`]), so the library shows the model without reading its geometry.

use crate::app::{App, Source};
use crate::start::StartPage;
use crate::stats::Stats;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

/// Frames drawn after the decision to leave before the picture is asked for: the menu page,
/// if it was open, gives way to the views first.
const SETTLE_FRAMES: u32 = 2;

/// Frames to wait for the picture before the model is left without one.
const PICTURE_FRAMES: u32 = 60;

/// Where the window goes once the picture of the model is taken.
enum Leave {
    /// The window closes.
    Close,
    /// Back to the model library.
    Library,
    /// Another model, in the same window.
    Switch(Box<Source>),
}

/// A model being left: its picture is on its way.
struct Leaving {
    leave: Leave,
    store: PathBuf,
    /// The active view's image rect, in points.
    rect: egui::Rect,
    frames: u32,
    shot: Option<Receiver<Arc<egui::ColorImage>>>,
}

/// The start page, then the window of the opened model.
pub struct Shell {
    appearance: Option<crate::appearance::Appearance>,
    start: StartPage,
    /// The open model window, if one was reached; the switch's tests read what replaced it.
    pub(crate) app: Option<App>,
    /// The window replaced or left in the previous pass: the output of that pass may still paint
    /// its textures, so it is released and dropped at the start of the next pass, whose output
    /// replaces it (egui paints only the last pass of a frame).
    retired: Option<App>,
    cli: PathBuf,
    samples: Option<u32>,
    stats: Arc<Stats>,
    error: Option<String>,
    leaving: Option<Leaving>,
    /// The picture is saved and the window may close now.
    closing: bool,
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
            leaving: None,
            closing: false,
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

impl Shell {
    /// Asks for the picture once the views have settled, saves it when it comes and then
    /// leaves the model; a picture that never comes leaves the model without one.
    fn continue_leaving(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(leaving) = &mut self.leaving else {
            return;
        };
        ctx.request_repaint();
        leaving.frames = leaving.frames.saturating_add(1);
        let Some(rx) = &leaving.shot else {
            if leaving.frames >= SETTLE_FRAMES {
                let (tx, rx) = std::sync::mpsc::channel();
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                    egui::ScreenshotCallback::new(move |image| {
                        let _ = tx.send(image);
                    }),
                ));
                leaving.shot = Some(rx);
            }
            return;
        };
        let image = match rx.try_recv() {
            Ok(image) => Some(image),
            Err(TryRecvError::Empty) if leaving.frames < PICTURE_FRAMES => return,
            Err(_) => None,
        };
        let Some(leaving) = self.leaving.take() else {
            return;
        };
        if let Some(image) = image {
            save_picture(&image, &leaving.store, leaving.rect, ctx.viewport_rect());
        }
        self.leave(ctx, frame, leaving.leave);
    }

    /// Leaves the open model: closes the window, returns to the library or opens another model.
    fn leave(&mut self, ctx: &egui::Context, frame: &eframe::Frame, leave: Leave) {
        match leave {
            Leave::Close => {
                self.closing = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Leave::Switch(source) => self.switch(ctx, frame, *source),
            Leave::Library => {
                if let Some(app) = &mut self.app {
                    self.appearance = app.appearance.take();
                }
                self.retired = self.app.take();
                self.start.return_to_models();
                ctx.send_viewport_cmd(egui::ViewportCommand::Title("BeamCraft".into()));
            }
        }
    }
}

/// Cuts the active view (`rect`, in points of `screen`) out of the window picture and saves it
/// as the model's `thumbnail.png`. A picture that cannot be saved is only reported on standard
/// error: leaving the model never waits on it or fails because of it.
fn save_picture(image: &egui::ColorImage, store: &Path, rect: egui::Rect, screen: egui::Rect) {
    let [width, height] = image.size;
    if screen.width() <= 0.0 || screen.height() <= 0.0 {
        return;
    }
    let sx = width as f32 / screen.width();
    let sy = height as f32 / screen.height();
    let px = |value: f32, scale: f32| (value * scale).round().max(0.0) as usize;
    let cut = [
        px(rect.left() - screen.left(), sx),
        px(rect.top() - screen.top(), sy),
        px(rect.right() - screen.left(), sx),
        px(rect.bottom() - screen.top(), sy),
    ];
    let Some(picture) = crate::thumbnail::crop(image, cut) else {
        return;
    };
    if let Err(error) = crate::thumbnail::save(store, &picture) {
        eprintln!("beamcraft-ui: {error}");
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
            if self.leaving.is_some() {
                self.continue_leaving(ui.ctx(), frame);
                return;
            }
            let close = !self.closing && ui.input(|i| i.viewport().close_requested());
            let leave = if close {
                Some(Leave::Close)
            } else if let Some(source) = app.take_open_request() {
                Some(Leave::Switch(Box::new(source)))
            } else if app.return_to_library {
                Some(Leave::Library)
            } else {
                None
            };
            let Some(leave) = leave else { return };
            match app.thumbnail_source() {
                Some((store, rect)) => {
                    if close {
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    }
                    app.show_views_for_thumbnail();
                    self.leaving = Some(Leaving {
                        leave,
                        store,
                        rect,
                        frames: 0,
                        shot: None,
                    });
                    ui.ctx().request_repaint();
                }
                None => self.leave(ui.ctx(), frame, leave),
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
