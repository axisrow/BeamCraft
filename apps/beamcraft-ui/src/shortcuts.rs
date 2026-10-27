// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Keyboard shortcuts of the five render modes (V16): one binding per mode, the default set
//! `Ctrl+1` to `Ctrl+5`, the user remaps them by editing `shortcuts.json` in the configuration
//! directory ([`default_dir`], the same directory the CLI keeps its `settings.json` in). A
//! missing file is the default bindings; a damaged one keeps the defaults and is reported, never
//! overwritten.

use beamcraft_engine::RenderMode;
use beamcraft_json::{Json, Limits};
use std::path::{Path, PathBuf};

/// File name of the shortcuts in the configuration directory.
pub const FILE_NAME: &str = "shortcuts.json";
/// Format version this module reads and writes.
pub const VERSION: u64 = 1;
/// Largest shortcuts file, in bytes.
pub const MAX_BYTES: usize = 4_096;
/// Environment variable that replaces the configuration directory (tests, portable installs).
pub const DIR_VARIABLE: &str = "BEAMCRAFT_CONFIG_DIR";

/// One binding: a key, with Ctrl when `ctrl` (the default set is Ctrl + a digit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    /// The key.
    pub key: egui::Key,
    /// Whether Ctrl is part of the binding.
    pub ctrl: bool,
}

impl Binding {
    /// Parses `Ctrl+1` or a bare `1`; the key is a digit `1` to `9`.
    pub fn parse(text: &str) -> Option<Self> {
        let (ctrl, rest) = match text.strip_prefix("Ctrl+") {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        let key = match rest {
            "1" => egui::Key::Num1,
            "2" => egui::Key::Num2,
            "3" => egui::Key::Num3,
            "4" => egui::Key::Num4,
            "5" => egui::Key::Num5,
            "6" => egui::Key::Num6,
            "7" => egui::Key::Num7,
            "8" => egui::Key::Num8,
            "9" => egui::Key::Num9,
            _ => return None,
        };
        Some(Self { key, ctrl })
    }

    /// The text of the file: `Ctrl+1` or a bare digit.
    pub fn as_text(self) -> String {
        let digit = match self.key {
            egui::Key::Num1 => "1",
            egui::Key::Num2 => "2",
            egui::Key::Num3 => "3",
            egui::Key::Num4 => "4",
            egui::Key::Num5 => "5",
            egui::Key::Num6 => "6",
            egui::Key::Num7 => "7",
            egui::Key::Num8 => "8",
            _ => "9",
        };
        if self.ctrl {
            format!("Ctrl+{digit}")
        } else {
            digit.to_owned()
        }
    }

    /// Whether the input state pressed this binding this frame. Shift and Alt are ignored; the
    /// Ctrl state must match, so a bare digit never fires while Ctrl is held.
    pub fn pressed(self, input: &egui::InputState) -> bool {
        input.key_pressed(self.key) && input.modifiers.ctrl == self.ctrl
    }
}

/// The five render-mode bindings, one per mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shortcuts {
    bindings: [(RenderMode, Binding); 5],
}

impl Default for Shortcuts {
    fn default() -> Self {
        let ctrl = |key| Binding { key, ctrl: true };
        Self {
            bindings: [
                (RenderMode::Wireframe, ctrl(egui::Key::Num1)),
                (RenderMode::Transparent, ctrl(egui::Key::Num2)),
                (RenderMode::Grey, ctrl(egui::Key::Num3)),
                (RenderMode::Shaded, ctrl(egui::Key::Num4)),
                (RenderMode::SelectedOnly, ctrl(egui::Key::Num5)),
            ],
        }
    }
}

impl Shortcuts {
    /// The binding of `mode`.
    pub fn get(self, mode: RenderMode) -> Binding {
        self.bindings
            .iter()
            .find(|(m, _)| *m == mode)
            .map(|(_, b)| *b)
            .unwrap_or(Binding {
                key: egui::Key::Num1,
                ctrl: true,
            })
    }

    /// Binds `mode` to `binding`; refused when another mode already holds that exact binding, so
    /// one key never drives two modes.
    pub fn set(&mut self, mode: RenderMode, binding: Binding) -> Result<(), &'static str> {
        if self
            .bindings
            .iter()
            .any(|(m, b)| *m != mode && *b == binding)
        {
            return Err("another render mode already uses this key");
        }
        if let Some(slot) = self.bindings.iter_mut().find(|(m, _)| *m == mode) {
            slot.1 = binding;
        }
        Ok(())
    }

    /// The mode whose binding the input state pressed this frame, if any.
    pub fn pressed(self, input: &egui::InputState) -> Option<RenderMode> {
        self.bindings
            .iter()
            .find(|(_, b)| b.pressed(input))
            .map(|(m, _)| *m)
    }

    /// The binding of `mode` as file text, for labels and tooltips.
    pub fn text(self, mode: RenderMode) -> String {
        self.get(mode).as_text()
    }

    /// The shortcuts as JSON, the shape of the file.
    pub fn encode(self) -> Vec<u8> {
        let text = |mode| Json::text(self.get(mode).as_text());
        Json::object([
            ("version", Json::uint(VERSION)),
            ("wireframe", text(RenderMode::Wireframe)),
            ("transparent", text(RenderMode::Transparent)),
            ("grey", text(RenderMode::Grey)),
            ("shaded", text(RenderMode::Shaded)),
            ("selected", text(RenderMode::SelectedOnly)),
        ])
        .encode(&limits())
        .unwrap_or_default()
    }

    /// Reads shortcuts bytes. Anything but a complete version-1 file of five digit bindings is
    /// refused.
    pub fn decode(bytes: &[u8]) -> Result<Self, &'static str> {
        let json = beamcraft_json::decode(bytes, &limits())
            .map_err(|_| "the shortcuts file is not valid")?;
        json.closed(&[
            "version",
            "wireframe",
            "transparent",
            "grey",
            "shaded",
            "selected",
        ])
        .map_err(|_| "the shortcuts file is not valid")?;
        let invalid = || "the shortcuts file is not valid";
        let version = json
            .get("version")
            .and_then(Json::unsigned)
            .map_err(|_| invalid())?;
        if version != VERSION {
            return Err("the shortcuts file has another version");
        }
        let mut shortcuts = Self::default();
        for mode in RenderMode::ALL {
            let text = json
                .get(mode.as_token())
                .and_then(Json::string)
                .map_err(|_| invalid())?;
            let binding = Binding::parse(text).ok_or("a shortcut is not Ctrl+1 to Ctrl+9")?;
            shortcuts
                .set(mode, binding)
                .map_err(|_| "two render modes share one key")?;
        }
        Ok(shortcuts)
    }

    /// The shortcuts of `config_dir` (`None` is [`default_dir`]): a missing file is the
    /// defaults; a damaged one returns the defaults and the reason.
    pub fn load(config_dir: Option<&Path>) -> (Self, Option<String>) {
        let Some(dir) = config_dir.map_or_else(default_dir, |d| Some(d.to_path_buf())) else {
            return (Self::default(), None);
        };
        let bytes = match std::fs::read(dir.join(FILE_NAME)) {
            Ok(bytes) if bytes.len() <= MAX_BYTES => bytes,
            Ok(_) => {
                return (
                    Self::default(),
                    Some("the shortcuts file is larger than 4 KiB; defaults kept".into()),
                );
            }
            // A missing file is a normal first run, not a failure.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Self::default(), None),
            Err(e) => {
                return (
                    Self::default(),
                    Some(format!(
                        "the shortcuts file could not be read ({e}); defaults kept"
                    )),
                );
            }
        };
        match Self::decode(&bytes) {
            Ok(shortcuts) => (shortcuts, None),
            Err(reason) => (Self::default(), Some(format!("{reason}; defaults kept"))),
        }
    }
}

fn limits() -> Limits {
    Limits {
        bytes: MAX_BYTES,
        depth: 3,
        tokens: 64,
        string_bytes: 128,
        scratch_bytes: 4_096,
    }
}

/// The configuration directory of the current user: [`DIR_VARIABLE`] when it is set, else
/// `%APPDATA%\BeamCraft` on Windows, `~/Library/Application Support/BeamCraft` on macOS and
/// `$XDG_CONFIG_HOME/beamcraft` or `~/.config/beamcraft` elsewhere. `None` when none is known.
/// The same directory the CLI's `settings.json` lives in.
pub fn default_dir() -> Option<PathBuf> {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if let Some(dir) = var(DIR_VARIABLE) {
        return Some(dir);
    }
    if cfg!(windows) {
        return var("APPDATA").map(|d| d.join("BeamCraft"));
    }
    if cfg!(target_os = "macos") {
        return var("HOME").map(|h| h.join("Library/Application Support/BeamCraft"));
    }
    var("XDG_CONFIG_HOME")
        .map(|d| d.join("beamcraft"))
        .or_else(|| var("HOME").map(|h| h.join(".config/beamcraft")))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn defaults_are_ctrl_1_to_ctrl_5_in_mode_order() {
        let s = Shortcuts::default();
        for (i, mode) in RenderMode::ALL.into_iter().enumerate() {
            let binding = s.get(mode);
            assert!(binding.ctrl);
            assert_eq!(binding.as_text(), format!("Ctrl+{}", i + 1));
            assert_eq!(s.text(mode), format!("Ctrl+{}", i + 1));
        }
        assert_eq!(s.get(RenderMode::Shaded), Binding::parse("Ctrl+4").unwrap());
    }

    #[test]
    fn bindings_round_trip_through_text_and_bytes() {
        for text in ["Ctrl+1", "Ctrl+9", "1", "9"] {
            let binding = Binding::parse(text).unwrap();
            assert_eq!(binding.as_text(), text);
        }
        for text in ["", "Ctrl+", "Ctrl+0", "Ctrl+F", "12", "A", "ctrl+1"] {
            assert!(Binding::parse(text).is_none(), "{text}");
        }
        let mut s = Shortcuts::default();
        s.set(RenderMode::Wireframe, Binding::parse("Ctrl+7").unwrap())
            .unwrap();
        assert_eq!(
            Shortcuts::decode(&s.encode()).unwrap(),
            s,
            "a remapped set survives a file round trip"
        );
        assert_eq!(
            Shortcuts::decode(&Shortcuts::default().encode()).unwrap(),
            Shortcuts::default()
        );
    }

    #[test]
    fn one_key_never_drives_two_modes_and_the_other_version_is_refused() {
        let mut s = Shortcuts::default();
        assert!(
            s.set(RenderMode::Grey, Binding::parse("Ctrl+1").unwrap())
                .is_err()
        );
        s.set(RenderMode::Grey, Binding::parse("1").unwrap())
            .unwrap();
        assert!(!s.get(RenderMode::Grey).ctrl);
        let mut bytes = Shortcuts::default().encode();
        let other = String::from_utf8(bytes.clone())
            .unwrap()
            .replace("\"version\":1", "\"version\":2");
        assert_eq!(
            Shortcuts::decode(other.as_bytes()).unwrap_err(),
            "the shortcuts file has another version"
        );
        bytes.truncate(bytes.len() - 2);
        assert!(Shortcuts::decode(&bytes).is_err());
        assert!(Shortcuts::decode(b"{}").is_err());
    }

    #[test]
    fn a_missing_file_is_the_defaults_and_a_damaged_one_is_reported() {
        let dir = std::env::temp_dir().join(format!(
            "beamcraft-shortcuts-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let (shortcuts, note) = Shortcuts::load(Some(&dir));
        assert_eq!(shortcuts, Shortcuts::default());
        assert!(note.is_none(), "a missing file is a normal first run");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE_NAME), Shortcuts::default().encode()).unwrap();
        let (shortcuts, note) = Shortcuts::load(Some(&dir));
        assert_eq!(shortcuts, Shortcuts::default());
        assert!(note.is_none());
        std::fs::write(dir.join(FILE_NAME), b"{").unwrap();
        let (shortcuts, note) = Shortcuts::load(Some(&dir));
        assert_eq!(shortcuts, Shortcuts::default());
        assert!(note.is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
