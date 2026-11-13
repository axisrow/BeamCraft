// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The operating system's folder dialog and file manager.
//!
//! Windows and macOS show their own dialog through `rfd` (no extra runtime). Linux has no
//! dependency-free system dialog, so the desktop's own dialog program is asked instead: `zenity`
//! (GNOME and most desktops), then `kdialog` (KDE). Without either the caller says so and the
//! path is typed. The dialog is modal: the window waits while it is open.

use std::path::{Path, PathBuf};

/// Why no folder could be chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogError {
    /// This system has no folder dialog that BeamCraft can show.
    Unavailable,
    /// The dialog program failed.
    Failed(String),
}

impl std::fmt::Display for DialogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => f.write_str(
                "no file or folder dialog is available here (install zenity or kdialog); type the path instead",
            ),
            Self::Failed(why) => write!(f, "the dialog failed ({why})"),
        }
    }
}

/// Asks for a folder, starting in `start` when it exists. `Ok(None)` means the dialog was
/// cancelled.
pub fn pick_folder(title: &str, start: Option<&Path>) -> Result<Option<PathBuf>, DialogError> {
    let start = start.filter(|p| p.is_dir());
    #[cfg(any(windows, target_os = "macos"))]
    {
        let mut dialog = rfd::FileDialog::new().set_title(title);
        if let Some(dir) = start {
            dialog = dialog.set_directory(dir);
        }
        Ok(dialog.pick_folder())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        desktop_dialog(title, start, FileChoice::Folder)
    }
}

/// Asks for one file (an IFC file, for the «Open IFC…» entry), starting in `start` when it
/// exists. `Ok(None)` means the dialog was cancelled.
pub fn pick_file(title: &str, start: Option<&Path>) -> Result<Option<PathBuf>, DialogError> {
    let start = start.filter(|p| p.is_dir());
    #[cfg(any(windows, target_os = "macos"))]
    {
        let mut dialog = rfd::FileDialog::new().set_title(title);
        if let Some(dir) = start {
            dialog = dialog.set_directory(dir);
        }
        Ok(dialog.pick_file())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        desktop_dialog(title, start, FileChoice::File)
    }
}

/// What the desktop's dialog program is asked to choose.
#[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileChoice {
    /// One existing file.
    File,
    /// One existing folder.
    Folder,
}

#[cfg(not(any(windows, target_os = "macos")))]
fn desktop_dialog(
    title: &str,
    start: Option<&Path>,
    choice: FileChoice,
) -> Result<Option<PathBuf>, DialogError> {
    use std::process::{Command, Stdio};
    let start = start.and_then(Path::to_str).unwrap_or("");
    let folder = choice == FileChoice::Folder;
    let programs: [(&str, Vec<String>); 2] = [
        (
            "zenity",
            vec![
                "--file-selection".into(),
                if folder {
                    "--directory".into()
                } else {
                    "--file-filter=*.ifc | *.ifc".into()
                },
                format!("--title={title}"),
                format!("--filename={start}/"),
            ],
        ),
        (
            "kdialog",
            vec![
                "--title".into(),
                title.into(),
                if folder {
                    "--getexistingdirectory".into()
                } else {
                    "--getopenfilename".into()
                },
                if start.is_empty() {
                    ".".into()
                } else {
                    start.into()
                },
            ],
        ),
    ];
    for (program, args) in programs {
        let output = match Command::new(program)
            .args(&args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
        {
            Ok(output) => output,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(DialogError::Failed(format!("{program}: {}", e.kind()))),
        };
        // Both programs exit with 1 when the dialog is cancelled.
        if output.status.code() == Some(1) {
            return Ok(None);
        }
        if !output.status.success() {
            return Err(DialogError::Failed(format!(
                "{program} exited with {}",
                output.status
            )));
        }
        return Ok(chosen(&output.stdout));
    }
    Err(DialogError::Unavailable)
}

/// The command that shows `dir` in the system's file manager: Explorer on Windows, Finder on
/// macOS, the desktop's `xdg-open` handler elsewhere.
pub fn folder_command(dir: &Path) -> std::process::Command {
    #[cfg(windows)]
    let program = "explorer";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(not(any(windows, target_os = "macos")))]
    let program = "xdg-open";
    let mut command = std::process::Command::new(program);
    command.arg(dir);
    command
}

/// Shows `dir` in the system's file manager without waiting for it. The reason is returned when
/// the folder is gone or the launcher cannot start; never panics.
pub fn open_folder(dir: &Path) -> Result<(), String> {
    use std::process::Stdio;
    if !dir.is_dir() {
        return Err("the model folder no longer exists".into());
    }
    let dir = std::path::absolute(dir)
        .map_err(|_| "the model folder's path cannot be resolved".to_owned())?;
    let mut command = folder_command(&dir);
    let program = command.get_program().to_string_lossy().into_owned();
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                format!("no file manager launcher was found ({program})")
            }
            kind => format!("the folder could not be opened ({kind})"),
        })?;
    // The exit status is not read: `explorer` returns 1 even when the folder opened. The child
    // is only waited for, so Unix leaves no zombie behind. When the thread cannot start, the
    // child is dropped unwaited and on Unix its zombie lasts until the window exits; the folder
    // still opens, so that is no failure.
    let _ = std::thread::Builder::new()
        .name("beamcraft-open-folder".into())
        .spawn(move || {
            let _ = child.wait();
        });
    Ok(())
}

/// The folder a dialog program printed: its first line, without the line end; `None` when empty.
#[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
fn chosen(stdout: &[u8]) -> Option<PathBuf> {
    let text = String::from_utf8_lossy(stdout);
    let line = text.lines().next()?.trim_end_matches(['\r', '\n']);
    (!line.is_empty()).then(|| PathBuf::from(line))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn reads_the_folder_a_dialog_printed() {
        assert_eq!(
            chosen(b"/home/a/Models\n"),
            Some(PathBuf::from("/home/a/Models"))
        );
        assert_eq!(
            chosen(b"/home/a/Model s\r\n"),
            Some(PathBuf::from("/home/a/Model s"))
        );
        assert_eq!(chosen(b"\n"), None);
        assert_eq!(chosen(b""), None);
    }

    #[test]
    fn builds_the_file_manager_command() {
        let command = folder_command(Path::new("/m/Model A"));
        #[cfg(windows)]
        assert_eq!(command.get_program(), "explorer");
        #[cfg(target_os = "macos")]
        assert_eq!(command.get_program(), "open");
        #[cfg(not(any(windows, target_os = "macos")))]
        assert_eq!(command.get_program(), "xdg-open");
        // A path with a space stays one argument.
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, [std::ffi::OsStr::new("/m/Model A")]);
    }

    #[test]
    fn a_missing_folder_is_refused_without_spawning() {
        let missing = std::env::temp_dir().join(format!(
            "beamcraft-open-folder-missing-{}",
            std::process::id()
        ));
        let error = open_folder(&missing).unwrap_err();
        assert!(error.contains("no longer exists"), "{error}");
    }
}
