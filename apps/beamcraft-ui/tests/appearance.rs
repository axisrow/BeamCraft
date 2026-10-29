// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The native theme selector writes and reloads real CLI settings in an isolated directory.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use beamcraft_ui::appearance::Appearance;
use beamcraft_ui::stats::Stats;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.0.is_absolute() && self.0.file_name().is_some());
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn frame(
    ctx: &egui::Context,
    appearance: &mut Appearance,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut consistent = false;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 600.0),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            appearance.ui(ui, true);
            consistent = ui.visuals().dark_mode == (ctx.theme() == egui::Theme::Dark)
                && ui.visuals().widgets.inactive.bg_fill
                    == beamcraft_ui::theme::Palette::current(ui).surface;
            ui.add(egui::TextEdit::singleline(&mut String::new()).hint_text("Theme probe"));
        },
    );
    output.textures_delta.clear();
    assert!(
        consistent,
        "The painted frame must use the same theme for backgrounds and widgets"
    );
    output
}

fn text_at(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
    output
        .shapes
        .iter()
        .find_map(|shape| {
            if let egui::Shape::Text(text) = &shape.shape
                && text.galley.text() == label
            {
                Some(text.pos + text.galley.size() / 2.0)
            } else {
                None
            }
        })
        .unwrap_or_else(|| panic!("native control not rendered: {label}"))
}

fn click(ctx: &egui::Context, appearance: &mut Appearance, pos: egui::Pos2) -> egui::FullOutput {
    frame(
        ctx,
        appearance,
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    )
    .drop_without_applying_deltas();
    frame(ctx, appearance, Vec::new()).drop_without_applying_deltas();
    frame(
        ctx,
        appearance,
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    )
    .drop_without_applying_deltas();
    frame(ctx, appearance, Vec::new())
}

fn settle(ctx: &egui::Context, appearance: &mut Appearance) -> egui::FullOutput {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let output = frame(ctx, appearance, Vec::new());
        if !appearance.busy() {
            return output;
        }
        assert!(Instant::now() < deadline, "appearance CLI did not finish");
        output.drop_without_applying_deltas();
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn native_selector_saves_and_reloads_all_preferences() {
    if matches!(
        std::env::var("BEAMCRAFT_THEME").as_deref(),
        Ok("light" | "dark")
    ) {
        eprintln!("SKIP appearance interaction: BEAMCRAFT_THEME explicitly overrides the selector");
        return;
    }
    let cli = Path::new(env!("CARGO_BIN_EXE_beamcraft-ui"))
        .parent()
        .unwrap()
        .join(format!("beamcraft{}", std::env::consts::EXE_SUFFIX));
    if !cli.is_file() {
        eprintln!("SKIP appearance interaction: build beamcraft-cli first");
        return;
    }
    let config = Fixture(
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("appearance-{}", std::process::id())),
    );
    std::fs::create_dir(&config.0).unwrap();
    let ctx = context();
    let stats = Arc::new(Stats::default());
    let mut appearance = Appearance::new(cli.clone(), Some(config.0.clone()), Arc::clone(&stats));
    let mut output = settle(&ctx, &mut appearance);
    for (before, label, expected) in [
        ("System", "Dark", egui::ThemePreference::Dark),
        ("Dark", "Light", egui::ThemePreference::Light),
        ("Light", "System", egui::ThemePreference::System),
    ] {
        let at = text_at(&output, before);
        output.drop_without_applying_deltas();
        let popup = click(&ctx, &mut appearance, at);
        let at = text_at(&popup, label);
        popup.drop_without_applying_deltas();
        click(&ctx, &mut appearance, at).drop_without_applying_deltas();
        output = settle(&ctx, &mut appearance);
        assert_eq!(ctx.options(|options| options.theme_preference), expected);
        let saved = Command::new(&cli)
            .args(["show-settings", "--config-dir"])
            .arg(&config.0)
            .output()
            .unwrap();
        assert!(saved.status.success());
        let settings = beamcraft_ui::start::parse_settings(Some(0), &saved.stdout).unwrap();
        assert_eq!(settings.theme, expected);
        let profile_saved = Command::new(&cli)
            .args([
                "set-settings",
                "--name",
                "UI Test",
                "--email",
                "ui@example.test",
                "--environment",
                "blank",
                "--skip-launcher",
                "true",
                "--config-dir",
            ])
            .arg(&config.0)
            .output()
            .unwrap();
        assert!(profile_saved.status.success());
        assert_eq!(
            beamcraft_ui::start::parse_settings(Some(0), &profile_saved.stdout)
                .unwrap()
                .theme,
            expected
        );
        let restart = context();
        let mut reloaded = Appearance::new(cli.clone(), Some(config.0.clone()), Arc::clone(&stats));
        settle(&restart, &mut reloaded).drop_without_applying_deltas();
        assert_eq!(
            restart.options(|options| options.theme_preference),
            expected
        );
    }
    output.drop_without_applying_deltas();
    let output = ctx.run_ui(Default::default(), |ui| appearance.titlebar(ui, true, true));
    let labels: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| {
            if let egui::Shape::Text(text) = &shape.shape {
                Some(text.galley.text().to_owned())
            } else {
                None
            }
        })
        .collect();
    output.drop_without_applying_deltas();
    assert!(labels.iter().any(|label| label == "BeamCraft"));
    assert!(
        !labels
            .iter()
            .any(|label| matches!(label.as_str(), "Theme" | "System" | "Light" | "Dark"))
    );
}

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    beamcraft_ui::theme::install(&ctx);
    let mut fonts = egui::FontDefinitions::empty();
    let inter = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts/inter/Inter-Regular.ttf"),
    )
    .unwrap();
    fonts
        .font_data
        .insert("Inter".into(), egui::FontData::from_owned(inter).into());
    for family in [
        egui::FontFamily::Proportional,
        egui::FontFamily::Monospace,
        beamcraft_ui::theme::bold_family(),
    ] {
        fonts.families.insert(family, vec!["Inter".into()]);
    }
    ctx.set_fonts(fonts);
    ctx
}
