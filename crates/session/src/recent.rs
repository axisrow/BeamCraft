// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The recent-project list: the settings' list of paths, kept honest against the folders.
//!
//! A path is remembered only when it is a project with a readable head, and a remembered project
//! that has definitely disappeared is dropped when the list is read. Both steps change the settings
//! file, so they run under the same write lock as every other settings change of the process.

use crate::Refusal;
use crate::project::{ProjectInfo, project_info, project_is_missing};
use crate::settings::{Settings, SettingsError};
use beamcraft_model::error::ErrorCode;
use std::path::Path;

/// One remembered project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentProject {
    /// When the project was last opened, in Unix milliseconds.
    pub opened_ms: u64,
    /// What reading the folder found.
    pub project: ProjectInfo,
}

/// The failure of a recent-project operation: the settings, or the project folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecentError {
    /// The settings file could not be read, changed or written.
    Settings(SettingsError),
    /// The project folder was refused.
    Project(Refusal),
}

impl RecentError {
    /// The stable F04 code.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        match self {
            Self::Settings(error) => error.code,
            Self::Project(error) => error.code,
        }
    }

    /// The fixed, safe message.
    #[must_use]
    pub const fn message(&self) -> &'static str {
        match self {
            Self::Settings(error) => error.message,
            Self::Project(error) => error.message,
        }
    }
}

impl From<SettingsError> for RecentError {
    fn from(error: SettingsError) -> Self {
        Self::Settings(error)
    }
}

impl From<Refusal> for RecentError {
    fn from(error: Refusal) -> Self {
        Self::Project(error)
    }
}

impl std::fmt::Display for RecentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code(), self.message())
    }
}

impl std::error::Error for RecentError {}

/// The recent projects of the settings in `dir`, newest first. A project whose folder has
/// definitely gone is dropped from the settings first, and the file is rewritten only when
/// something was dropped.
///
/// # Errors
/// The settings file is damaged, too large or cannot be written; a remembered path that has no
/// UTF-8 folder name is refused.
pub fn recent_projects(dir: &Path) -> Result<Vec<RecentProject>, RecentError> {
    let settings = Settings::update(dir, |settings| {
        let before = settings.recent.len();
        settings.recent.retain(|r| !project_is_missing(&r.path));
        Ok(settings.recent.len() != before)
    })?;
    settings
        .recent
        .iter()
        .map(|r| {
            Ok(RecentProject {
                opened_ms: r.opened_ms,
                project: project_info(&r.path)?,
            })
        })
        .collect()
}

/// Remembers the project at `path` as opened at `now_ms`: it moves to the front of the list.
///
/// # Errors
/// A folder that is not a project with a readable head is `NOT_FOUND`, and the list is left as it
/// was. A damaged settings file is refused and never overwritten.
pub fn remember_project(dir: &Path, path: &Path, now_ms: u64) -> Result<ProjectInfo, RecentError> {
    let info = project_info(path)?;
    if !info.state.is_ready() {
        return Err(Refusal::new(
            ErrorCode::NotFound,
            "the folder is not a project with a readable head",
        )
        .into());
    }
    Settings::update(dir, |settings| {
        settings.touch_recent(info.path.clone(), now_ms)?;
        Ok(true)
    })?;
    Ok(info)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("session-recent-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_folder_that_is_not_a_project_is_not_remembered() {
        let config = scratch("not-project-config");
        let folder = scratch("not-project-folder");
        let error = remember_project(&config, &folder, 5).unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
        // The settings file was never created or changed.
        assert!(Settings::load(&config).unwrap().recent.is_empty());
        let _ = fs::remove_dir_all(&config);
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn projects_that_have_disappeared_are_dropped_from_the_list() {
        let config = scratch("prune-config");
        let gone = scratch("prune-gone").join("Hall");
        let mut settings = Settings::default();
        settings.touch_recent(gone, 1).unwrap();
        settings.save(&config).unwrap();
        assert!(recent_projects(&config).unwrap().is_empty());
        assert!(Settings::load(&config).unwrap().recent.is_empty());
        let _ = fs::remove_dir_all(&config);
    }

    #[test]
    fn a_damaged_settings_file_is_refused_and_kept() {
        let config = scratch("damaged");
        fs::write(config.join("settings.json"), b"{ not json").unwrap();
        assert!(recent_projects(&config).is_err());
        assert_eq!(
            fs::read(config.join("settings.json")).unwrap(),
            b"{ not json"
        );
        let _ = fs::remove_dir_all(&config);
    }

    #[test]
    fn updates_from_two_threads_do_not_lose_each_other() {
        let config = scratch("threads");
        let handles: Vec<_> = (0..8_u64)
            .map(|n| {
                let config = config.clone();
                std::thread::spawn(move || {
                    Settings::update(&config, |s| {
                        s.touch_recent(PathBuf::from(format!("/p/{n}")), n)?;
                        Ok(true)
                    })
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap().unwrap();
        }
        assert_eq!(Settings::load(&config).unwrap().recent.len(), 8);
        let _ = fs::remove_dir_all(&config);
    }
}
