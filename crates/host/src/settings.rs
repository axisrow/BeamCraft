// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Per-user application settings: the author profile, the environment chosen at start, whether the
//! start window is skipped, the folder of models and the recently opened projects.
//!
//! The settings are one bounded JSON file, `settings.json`, in the user's configuration directory
//! ([`default_dir`]). They are application state of one person on one computer, never project
//! data: no model is read from or written to them, and a project never depends on them.
//!
//! The profile gives a person one author identity, a UUIDv7 made once when the profile is first
//! saved and kept when the name or e-mail address changes. Commands record that identity as their
//! actor; the name and the address are labels for people, never a credential or a permission.
//!
//! A file that is missing reads as the default settings. A file that is present but damaged,
//! too large or of another version is refused with a typed error and never overwritten silently:
//! the caller decides. A save writes a complete temporary file next to the target and renames it
//! over the target, so a reader sees the old or the new file.

use beamcraft_json::{Json, Limits};
use beamcraft_model::ErrorCode;
use beamcraft_model::id::Uuid7;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// File name of the settings in the configuration directory.
pub const FILE_NAME: &str = "settings.json";
/// Format version this module reads and writes.
pub const VERSION: u64 = 1;
/// Largest settings file, in bytes.
pub const MAX_BYTES: usize = 65_536;
/// Most recent projects kept; older ones are dropped.
pub const MAX_RECENT: usize = 20;
/// Longest author name, in characters.
pub const MAX_NAME_CHARS: usize = 128;
/// Longest e-mail address, in bytes (RFC 5321 path limit).
pub const MAX_EMAIL_BYTES: usize = 254;
/// Longest environment identifier, in bytes.
pub const MAX_ENVIRONMENT_BYTES: usize = 128;
/// Longest stored path, in bytes.
pub const MAX_PATH_BYTES: usize = 1_024;

/// Environment variable that replaces the configuration directory (tests, portable installs).
pub const DIR_VARIABLE: &str = "BEAMCRAFT_CONFIG_DIR";

/// A refusal with its stable code and a fixed message; no path or OS text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingsError {
    /// Stable F04 code.
    pub code: ErrorCode,
    /// Fixed ASCII explanation.
    pub message: &'static str,
}

impl SettingsError {
    const fn new(code: ErrorCode, message: &'static str) -> Self {
        Self { code, message }
    }
}

fn invalid(message: &'static str) -> SettingsError {
    SettingsError::new(ErrorCode::InvalidArgument, message)
}

fn damaged(message: &'static str) -> SettingsError {
    SettingsError::new(ErrorCode::IntegrityError, message)
}

fn io_error(error: &io::Error, message: &'static str) -> SettingsError {
    let code = match error.kind() {
        io::ErrorKind::NotFound => ErrorCode::NotFound,
        io::ErrorKind::PermissionDenied => ErrorCode::PermissionDenied,
        _ => ErrorCode::BackendUnavailable,
    };
    SettingsError::new(code, message)
}

/// The author of the edits made on this computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// Author identity recorded as the actor of every command; made once, never changed.
    pub actor: Uuid7,
    /// Display name, 1 to [`MAX_NAME_CHARS`] characters.
    pub name: String,
    /// E-mail address, a label only.
    pub email: String,
}

/// One recently opened project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recent {
    /// The project's store directory, as it was opened.
    pub path: PathBuf,
    /// When it was last opened, in Unix milliseconds.
    pub opened_ms: u64,
}

/// The application appearance preference.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ThemePreference {
    /// Follow the operating system, including changes while the application is open.
    #[default]
    System,
    /// Always use light surfaces.
    Light,
    /// Always use dark surfaces.
    Dark,
}

impl ThemePreference {
    /// The stable settings and CLI spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    /// Parses an explicit preference; unknown values are refused.
    pub fn parse(value: &str) -> Result<Self, SettingsError> {
        match value {
            "system" => Ok(Self::System),
            "light" => Ok(Self::Light),
            "dark" => Ok(Self::Dark),
            _ => Err(invalid("the theme must be system, light or dark")),
        }
    }
}

/// The settings of one user.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    /// Appearance; older settings files without this field follow the system.
    pub theme: ThemePreference,
    /// The author profile, once entered.
    pub profile: Option<Profile>,
    /// The identifier of the environment chosen at start.
    pub environment: Option<String>,
    /// Open the start page directly with the saved environment and profile.
    pub skip_launcher: bool,
    /// The folder listed under "All models".
    pub models_dir: Option<PathBuf>,
    /// Recently opened projects, newest first, at most [`MAX_RECENT`], no path twice.
    pub recent: Vec<Recent>,
}

/// Checks a display name: trimmed, 1 to [`MAX_NAME_CHARS`] characters, no control character.
pub fn check_name(name: &str) -> Result<(), SettingsError> {
    let count = name.chars().count();
    if count == 0 || count > MAX_NAME_CHARS || name.trim() != name {
        return Err(invalid(
            "the name must be 1 to 128 characters without outer spaces",
        ));
    }
    if name.chars().any(char::is_control) {
        return Err(invalid("the name contains a control character"));
    }
    Ok(())
}

/// Checks the form of an e-mail address: one `@` between a non-empty local part and a domain of
/// at least two dot-separated labels, no space, control character or `<>()[],;:"\` and at most
/// [`MAX_EMAIL_BYTES`] bytes. Whether the address exists is never checked.
pub fn check_email(email: &str) -> Result<(), SettingsError> {
    let refuse = || invalid("the e-mail address is not of the form name@domain.tld");
    if email.is_empty() || email.len() > MAX_EMAIL_BYTES {
        return Err(refuse());
    }
    if email
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || "<>()[],;:\"\\".contains(c))
    {
        return Err(refuse());
    }
    let mut parts = email.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(refuse());
    };
    let labels: Vec<&str> = domain.split('.').collect();
    if local.is_empty() || labels.len() < 2 || labels.iter().any(|l| l.is_empty()) {
        return Err(refuse());
    }
    Ok(())
}

/// Checks an environment identifier: 1 to [`MAX_ENVIRONMENT_BYTES`] bytes of ASCII letters,
/// digits, `_`, `-` and `.`.
pub fn check_environment(id: &str) -> Result<(), SettingsError> {
    let ok = !id.is_empty()
        && id.len() <= MAX_ENVIRONMENT_BYTES
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'));
    if ok {
        Ok(())
    } else {
        Err(invalid(
            "the environment id is not 1 to 128 of A-Z a-z 0-9 _ - .",
        ))
    }
}

fn path_text(path: &Path) -> Result<&str, SettingsError> {
    path.to_str()
        .filter(|t| !t.is_empty() && t.len() <= MAX_PATH_BYTES && !t.chars().any(char::is_control))
        .ok_or_else(|| invalid("the path is not 1 to 1024 bytes of UTF-8 text"))
}

impl Settings {
    /// Checks every field; [`Settings::encode`] and [`Settings::decode`] both apply it.
    pub fn validate(&self) -> Result<(), SettingsError> {
        if let Some(profile) = &self.profile {
            check_name(&profile.name)?;
            check_email(&profile.email)?;
        }
        if let Some(id) = &self.environment {
            check_environment(id)?;
        }
        if let Some(dir) = &self.models_dir {
            path_text(dir)?;
        }
        if self.recent.len() > MAX_RECENT {
            return Err(invalid("too many recent projects"));
        }
        for (i, recent) in self.recent.iter().enumerate() {
            path_text(&recent.path)?;
            if self
                .recent
                .iter()
                .skip(i + 1)
                .any(|r| r.path == recent.path)
            {
                return Err(invalid("a recent project is listed twice"));
            }
        }
        Ok(())
    }

    /// Records `path` as opened at `now_ms`: it moves to the front, and the list keeps at most
    /// [`MAX_RECENT`] entries.
    pub fn touch_recent(&mut self, path: PathBuf, now_ms: u64) -> Result<(), SettingsError> {
        path_text(&path)?;
        self.recent.retain(|r| r.path != path);
        self.recent.insert(
            0,
            Recent {
                path,
                opened_ms: now_ms,
            },
        );
        self.recent.truncate(MAX_RECENT);
        Ok(())
    }

    /// Sets the name and address of the profile. The first profile takes the identity `fresh`
    /// makes; an existing one keeps its identity, so earlier edits stay attributed to it.
    pub fn set_profile(
        &mut self,
        name: &str,
        email: &str,
        fresh: impl FnOnce() -> Result<Uuid7, SettingsError>,
    ) -> Result<(), SettingsError> {
        check_name(name)?;
        check_email(email)?;
        let actor = match &self.profile {
            Some(profile) => profile.actor,
            None => fresh()?,
        };
        self.profile = Some(Profile {
            actor,
            name: name.to_owned(),
            email: email.to_owned(),
        });
        Ok(())
    }

    /// The settings as JSON, the shape of the file and of the CLI reply.
    pub fn to_json(&self) -> Result<Json, SettingsError> {
        self.validate()?;
        let text_or_null = |value: Option<&str>| value.map_or(Json::Null, Json::text);
        let profile = match &self.profile {
            Some(p) => Json::object([
                ("actor_id", Json::text(p.actor.to_string())),
                ("name", Json::text(p.name.clone())),
                ("email", Json::text(p.email.clone())),
            ]),
            None => Json::Null,
        };
        let mut recent = Vec::with_capacity(self.recent.len());
        for r in &self.recent {
            recent.push(Json::object([
                ("path", Json::text(path_text(&r.path)?)),
                ("opened_ms", Json::uint(r.opened_ms)),
            ]));
        }
        let models_dir = match &self.models_dir {
            Some(dir) => Json::text(path_text(dir)?),
            None => Json::Null,
        };
        Ok(Json::object([
            ("version", Json::uint(VERSION)),
            ("theme", Json::text(self.theme.as_str())),
            ("profile", profile),
            ("environment", text_or_null(self.environment.as_deref())),
            ("skip_launcher", Json::Bool(self.skip_launcher)),
            ("models_dir", models_dir),
            ("recent", Json::Array(recent)),
        ]))
    }

    /// The settings file bytes.
    pub fn encode(&self) -> Result<Vec<u8>, SettingsError> {
        let bytes = self.to_json()?.encode(&limits()).map_err(|_| {
            SettingsError::new(ErrorCode::LimitExceeded, "the settings are too large")
        })?;
        Ok(bytes)
    }

    /// Reads settings bytes. Anything but a complete version-1 file is refused.
    pub fn decode(bytes: &[u8]) -> Result<Self, SettingsError> {
        let shape = |_| damaged("the settings file is not valid");
        let json = beamcraft_json::decode(bytes, &limits()).map_err(shape)?;
        let mut keys = vec![
            "version",
            "profile",
            "environment",
            "skip_launcher",
            "models_dir",
            "recent",
        ];
        if json.get("theme").is_ok() {
            keys.push("theme");
        }
        json.closed(&keys).map_err(shape)?;
        if json
            .get("version")
            .and_then(Json::unsigned)
            .map_err(shape)?
            != VERSION
        {
            return Err(SettingsError::new(
                ErrorCode::UnsupportedVersion,
                "the settings file has another version",
            ));
        }
        let optional_text = |key: &str| -> Result<Option<String>, SettingsError> {
            match json.get(key).map_err(shape)? {
                Json::Null => Ok(None),
                value => Ok(Some(value.string().map_err(shape)?.to_owned())),
            }
        };
        let profile = match json.get("profile").map_err(shape)? {
            Json::Null => None,
            p => {
                p.closed(&["actor_id", "name", "email"]).map_err(shape)?;
                let text = |key: &str| p.get(key).and_then(Json::string).map_err(shape);
                Some(Profile {
                    actor: Uuid7::parse(text("actor_id")?)
                        .map_err(|_| damaged("the settings file is not valid"))?,
                    name: text("name")?.to_owned(),
                    email: text("email")?.to_owned(),
                })
            }
        };
        let mut recent = Vec::new();
        for r in json.get("recent").and_then(Json::array).map_err(shape)? {
            r.closed(&["path", "opened_ms"]).map_err(shape)?;
            recent.push(Recent {
                path: PathBuf::from(r.get("path").and_then(Json::string).map_err(shape)?),
                opened_ms: r.get("opened_ms").and_then(Json::unsigned).map_err(shape)?,
            });
        }
        let settings = Self {
            theme: match json.get("theme") {
                Ok(value) => ThemePreference::parse(value.string().map_err(shape)?)
                    .map_err(|_| damaged("the settings theme is not valid"))?,
                Err(_) => ThemePreference::System,
            },
            profile,
            environment: optional_text("environment")?,
            skip_launcher: json
                .get("skip_launcher")
                .and_then(Json::boolean)
                .map_err(shape)?,
            models_dir: optional_text("models_dir")?.map(PathBuf::from),
            recent,
        };
        settings
            .validate()
            .map_err(|_| damaged("the settings file is not valid"))?;
        Ok(settings)
    }

    /// Reads the settings of `dir`; a missing file is the default settings.
    pub fn load(dir: &Path) -> Result<Self, SettingsError> {
        let path = dir.join(FILE_NAME);
        let file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(io_error(&e, "the settings file could not be opened")),
        };
        let mut bytes = Vec::new();
        file.take(u64::try_from(MAX_BYTES).unwrap_or(u64::MAX) + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| io_error(&e, "the settings file could not be read"))?;
        if bytes.len() > MAX_BYTES {
            return Err(SettingsError::new(
                ErrorCode::LimitExceeded,
                "the settings file is larger than 64 KiB",
            ));
        }
        Self::decode(&bytes)
    }

    /// Writes the settings into `dir`, creating the directory when it is missing.
    pub fn save(&self, dir: &Path) -> Result<(), SettingsError> {
        let bytes = self.encode()?;
        fs::create_dir_all(dir)
            .map_err(|e| io_error(&e, "the settings directory could not be created"))?;
        let target = dir.join(FILE_NAME);
        let temporary = dir.join(format!(".{FILE_NAME}.tmp-{}", std::process::id()));
        let written = fs::File::create(&temporary).and_then(|mut file| {
            file.write_all(&bytes)?;
            file.sync_all()
        });
        if let Err(e) = written.and_then(|()| fs::rename(&temporary, &target)) {
            // The temporary file is ours; the target is untouched.
            let _ = fs::remove_file(&temporary);
            return Err(io_error(&e, "the settings file could not be written"));
        }
        Ok(())
    }
}

fn limits() -> Limits {
    Limits {
        bytes: MAX_BYTES,
        depth: 4,
        tokens: 512,
        string_bytes: MAX_PATH_BYTES * 4,
        scratch_bytes: 1_048_576,
    }
}

/// The configuration directory of the current user: [`DIR_VARIABLE`] when it is set, else
/// `%APPDATA%\BeamCraft` on Windows, `~/Library/Application Support/BeamCraft` on macOS and
/// `$XDG_CONFIG_HOME/beamcraft` or `~/.config/beamcraft` elsewhere. `None` when none is known.
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

    const ACTOR: &str = "0199e7a0-0000-7000-8000-000000000001";

    fn actor() -> Uuid7 {
        Uuid7::parse(ACTOR).unwrap()
    }

    fn sample() -> Settings {
        let mut s = Settings::default();
        s.set_profile("Ana Pérez", "ana@example.cl", || Ok(actor()))
            .unwrap();
        s.environment = Some("blank".into());
        s.skip_launcher = true;
        s.models_dir = Some(PathBuf::from("/models"));
        s.touch_recent(PathBuf::from("/models/a"), 10).unwrap();
        s
    }

    #[test]
    fn round_trips_through_bytes() {
        let s = sample();
        assert_eq!(Settings::decode(&s.encode().unwrap()).unwrap(), s);
        let empty = Settings::default();
        assert_eq!(Settings::decode(&empty.encode().unwrap()).unwrap(), empty);
    }

    #[test]
    fn appearance_is_optional_for_old_files_and_strict_when_present() {
        let legacy = br#"{"version":1,"profile":null,"environment":null,"skip_launcher":false,"models_dir":null,"recent":[]}"#;
        assert_eq!(
            Settings::decode(legacy).unwrap().theme,
            ThemePreference::System
        );
        for theme in [
            ThemePreference::System,
            ThemePreference::Light,
            ThemePreference::Dark,
        ] {
            let settings = Settings {
                theme,
                ..Settings::default()
            };
            assert_eq!(
                Settings::decode(&settings.encode().unwrap()).unwrap(),
                settings
            );
        }
        let unknown = String::from_utf8(legacy.to_vec())
            .unwrap()
            .replace("\"version\":1", "\"version\":1,\"theme\":\"automatic\"");
        assert_eq!(
            Settings::decode(unknown.as_bytes()).unwrap_err().code,
            ErrorCode::IntegrityError
        );
        let wrong_type = unknown.replace("\"automatic\"", "null");
        assert!(Settings::decode(wrong_type.as_bytes()).is_err());
    }

    #[test]
    fn keeps_the_actor_when_the_profile_changes() {
        let mut s = sample();
        s.set_profile("Ana", "ana@example.com", || {
            Err(invalid("a second identity must not be made"))
        })
        .unwrap();
        let p = s.profile.unwrap();
        assert_eq!((p.actor, p.email.as_str()), (actor(), "ana@example.com"));
    }

    #[test]
    fn recent_moves_to_the_front_and_is_bounded() {
        let mut s = Settings::default();
        for i in 0..30 {
            s.touch_recent(PathBuf::from(format!("/m/{i}")), i).unwrap();
        }
        s.touch_recent(PathBuf::from("/m/15"), 99).unwrap();
        assert_eq!(s.recent.len(), MAX_RECENT);
        assert_eq!(s.recent[0].path, PathBuf::from("/m/15"));
        assert_eq!(s.recent[1].path, PathBuf::from("/m/29"));
        assert_eq!(
            s.recent.iter().filter(|r| r.path.ends_with("15")).count(),
            1
        );
    }

    #[test]
    fn refuses_bad_profiles() {
        for email in [
            "", "a", "a@b", "@b.c", "a@.c", "a@b.", "a b@c.d", "a@b@c.d", "<a@b.c>",
        ] {
            assert!(check_email(email).is_err(), "{email}");
        }
        for email in ["a@b.cl", "first.last+tag@sub.example.com", "иван@пример.рф"] {
            assert!(check_email(email).is_ok(), "{email}");
        }
        for name in ["", " a", "a ", "a\nb", &"x".repeat(129)] {
            assert!(check_name(name).is_err(), "{name:?}");
        }
        assert!(check_environment("blank").is_ok());
        assert!(check_environment("../x").is_err());
    }

    #[test]
    fn refuses_damaged_or_other_files() {
        let mut bytes = sample().encode().unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        let other = text.replace("\"version\":1", "\"version\":2");
        assert_eq!(
            Settings::decode(other.as_bytes()).unwrap_err().code,
            ErrorCode::UnsupportedVersion
        );
        let bad = text.replace(ACTOR, "not-an-id");
        assert_eq!(
            Settings::decode(bad.as_bytes()).unwrap_err().code,
            ErrorCode::IntegrityError
        );
        bytes.truncate(bytes.len() - 1);
        assert!(Settings::decode(&bytes).is_err());
        assert!(Settings::decode(b"{}").is_err());
    }

    #[test]
    fn saves_and_loads_a_directory() {
        let dir = std::env::temp_dir().join(format!(
            "beamcraft-settings-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(Settings::load(&dir).unwrap(), Settings::default());
        let s = sample();
        s.save(&dir).unwrap();
        assert_eq!(Settings::load(&dir).unwrap(), s);
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            1,
            "no temporary file left"
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
