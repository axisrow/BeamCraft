// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Atomic publication of one output file (E01 section 14, "Output publication").
//!
//! The bytes are written to a temporary file in the destination directory, flushed and synced,
//! and installed under the final name only after the caller's writer succeeded. An existing file
//! is not replaced unless the caller asks for it; a symbolic link or Windows reparse point at the
//! output name or at its parent directory is refused. A failure removes the temporary file and
//! leaves no output. The path is an I/O-edge argument and authorises nothing else.

use crate::storage::is_link;
use beamcraft_model::error::ErrorCode;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Why an output could not be published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputError {
    /// The F04 code of the failure.
    pub code: ErrorCode,
    /// A fixed message that never echoes the path.
    pub message: &'static str,
}

impl OutputError {
    const fn new(code: ErrorCode, message: &'static str) -> Self {
        Self { code, message }
    }
}

impl std::fmt::Display for OutputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for OutputError {}

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Temporary(PathBuf);

impl Drop for Temporary {
    fn drop(&mut self) {
        // Best effort: the file may already be installed or gone.
        let _ = fs::remove_file(&self.0);
    }
}

fn unsafe_directory() -> OutputError {
    OutputError::new(
        ErrorCode::InvalidArgument,
        "the output directory, or one of its ancestors, is a link or not a directory",
    )
}

/// Whether the text names a network share or a device namespace (`\\server\share`,
/// `\\?\UNC\...`, `\\.\COM1`, or the same with forward slashes). Local verbatim drives
/// (`\\?\C:\...`) are not network paths.
pub fn is_network_or_device_path(text: &str) -> bool {
    let unified: String = text
        .chars()
        .map(|c| if c == '/' { '\\' } else { c })
        .collect();
    let Some(rest) = unified.strip_prefix("\\\\") else {
        return false;
    };
    // `\\?\C:\` is a verbatim local disk; anything else after two separators is not local.
    let local_disk = rest.strip_prefix("?\\").is_some_and(|r| {
        let mut chars = r.chars();
        matches!((chars.next(), chars.next()), (Some(d), Some(':')) if d.is_ascii_alphabetic())
    });
    !local_disk
}

/// A destination after validation: the canonical directory it lies in and its file name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    /// The canonical (link-free) directory.
    pub directory: PathBuf,
    /// The file name.
    pub name: OsString,
}

impl Destination {
    /// Whether two destinations name the same file (the comparison ignores case where the host
    /// file systems usually do).
    pub fn same_file(&self, other: &Self) -> bool {
        if self.directory != other.directory {
            return false;
        }
        if cfg!(any(windows, target_os = "macos")) {
            self.name.to_string_lossy().to_lowercase()
                == other.name.to_string_lossy().to_lowercase()
        } else {
            self.name == other.name
        }
    }
}

/// Validates a destination without creating anything: a local path (no network share or device
/// namespace), a file name, and a directory whose every component, from the first one the caller
/// wrote down, is an ordinary directory (no symbolic link and no reparse point at any level, so
/// a link cannot redirect the write). Relative paths are taken from the current directory, which
/// is the caller's own choice. An existing target must be a regular file, not a link.
pub fn resolve_destination(target: &Path) -> Result<Destination, OutputError> {
    let invalid = |m| OutputError::new(ErrorCode::InvalidArgument, m);
    if is_network_or_device_path(&target.to_string_lossy()) && cfg!(windows) {
        return Err(invalid("a network or device path is not a local output"));
    }
    #[cfg(windows)]
    {
        use std::path::Prefix;
        if let Some(Component::Prefix(p)) = target.components().next()
            && !matches!(p.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        {
            return Err(invalid("a network or device path is not a local output"));
        }
    }
    let name = target
        .file_name()
        .filter(|n| !n.is_empty() && n.len() <= 200 && n.to_str().is_some())
        .ok_or(invalid("the output path has no usable file name"))?
        .to_owned();
    let parent = match target.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let mut prefix = PathBuf::new();
    for component in parent.components() {
        match component {
            Component::CurDir => continue,
            Component::ParentDir => {
                // Every directory walked so far was checked, so the lexical parent is the real one.
                if !prefix.pop() {
                    prefix.push("..");
                }
            }
            other => prefix.push(other),
        }
        if prefix.as_os_str().is_empty() {
            continue;
        }
        let meta = fs::symlink_metadata(&prefix).map_err(|_| {
            OutputError::new(
                ErrorCode::InvalidArgument,
                "the output directory does not exist",
            )
        })?;
        if is_link(&meta) || !meta.is_dir() {
            return Err(unsafe_directory());
        }
    }
    let directory = fs::canonicalize(if prefix.as_os_str().is_empty() {
        Path::new(".")
    } else {
        prefix.as_path()
    })
    .map_err(|_| unsafe_directory())?;
    match fs::symlink_metadata(target) {
        Ok(meta) => {
            if is_link(&meta) || !meta.is_file() {
                return Err(invalid("the output name is a link or not a regular file"));
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(_) => {
            return Err(OutputError::new(
                ErrorCode::BackendUnavailable,
                "the output name could not be examined",
            ));
        }
    }
    Ok(Destination { directory, name })
}

/// A complete, synced temporary file waiting to be installed under its final name. Dropping it
/// removes the temporary file and leaves the destination as it was.
pub struct Staged {
    temporary: Temporary,
    destination: Destination,
    overwrite: bool,
    existed: bool,
}

/// A file installed by [`Staged::install`].
pub struct Installed {
    path: PathBuf,
    created: bool,
}

impl Installed {
    /// Removes the file again if the install created it. A replaced file cannot be restored, so
    /// that case is left as it is.
    pub fn undo(self) {
        if self.created {
            let _ = fs::remove_file(&self.path);
        }
    }
}

impl Staged {
    /// The validated destination.
    pub fn destination(&self) -> &Destination {
        &self.destination
    }

    /// Installs the file under its final name: a rename with `overwrite`, otherwise a
    /// non-replacing hard link (a file system that cannot make one fails with
    /// `BACKEND_UNAVAILABLE`; there is no rename fallback that could replace a file created in
    /// the meantime).
    pub fn install(self) -> Result<Installed, OutputError> {
        let unavailable = |_: std::io::Error| {
            OutputError::new(
                ErrorCode::BackendUnavailable,
                "the output could not be written",
            )
        };
        let path = self.destination.directory.join(&self.destination.name);
        if self.overwrite {
            fs::rename(&self.temporary.0, &path).map_err(unavailable)?;
        } else {
            match fs::hard_link(&self.temporary.0, &path) {
                Ok(()) => {}
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                    return Err(OutputError::new(
                        ErrorCode::InvalidArgument,
                        "the output file exists and overwrite was not requested",
                    ));
                }
                Err(e) => return Err(unavailable(e)),
            }
        }
        Ok(Installed {
            path,
            created: !self.existed,
        })
    }
}

/// Validates `target` and writes a complete temporary file for it with `write`, without
/// installing anything. The caller can stage several outputs and install them only when every
/// one of them was written.
pub fn stage_file<F>(target: &Path, overwrite: bool, write: F) -> Result<Staged, OutputError>
where
    F: FnOnce(&mut File) -> Result<(), OutputError>,
{
    let destination = resolve_destination(target)?;
    let path = destination.directory.join(&destination.name);
    let existed = fs::symlink_metadata(&path).is_ok();
    if existed && !overwrite {
        return Err(OutputError::new(
            ErrorCode::InvalidArgument,
            "the output file exists and overwrite was not requested",
        ));
    }
    let unavailable = |_: std::io::Error| {
        OutputError::new(
            ErrorCode::BackendUnavailable,
            "the output could not be written",
        )
    };
    let temporary_path = destination.directory.join(format!(
        ".{}.tmp-{}-{}",
        destination.name.to_string_lossy(),
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_path)
        .map_err(unavailable)?;
    let temporary = Temporary(temporary_path);
    write(&mut file)?;
    file.sync_all().map_err(unavailable)?;
    drop(file);
    Ok(Staged {
        temporary,
        destination,
        overwrite,
        existed,
    })
}

/// Publishes the file `target` with the bytes `write` produces: [`stage_file`] then
/// [`Staged::install`].
pub fn publish_file<F>(target: &Path, overwrite: bool, write: F) -> Result<(), OutputError>
where
    F: FnOnce(&mut File) -> Result<(), OutputError>,
{
    stage_file(target, overwrite, write)?.install().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "beamcraft-output-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(text: &'static str) -> impl FnOnce(&mut File) -> Result<(), OutputError> {
        move |f| {
            f.write_all(text.as_bytes())
                .map_err(|_| OutputError::new(ErrorCode::BackendUnavailable, "write"))
        }
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect()
    }

    #[test]
    fn a_new_file_is_installed_whole_and_no_temporary_remains() {
        let dir = scratch("new");
        let target = dir.join("a.ifc");
        publish_file(&target, false, put("hello")).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "hello");
        assert_eq!(leftovers(&dir), ["a.ifc"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_existing_file_is_kept_unless_overwrite_is_asked() {
        let dir = scratch("exists");
        let target = dir.join("a.ifc");
        publish_file(&target, false, put("one")).unwrap();
        let refused = publish_file(&target, false, put("two")).unwrap_err();
        assert_eq!(refused.code, ErrorCode::InvalidArgument);
        assert_eq!(fs::read_to_string(&target).unwrap(), "one");
        publish_file(&target, true, put("two")).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "two");
        assert_eq!(leftovers(&dir), ["a.ifc"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failing_writer_leaves_no_output_and_no_temporary() {
        let dir = scratch("fail");
        let target = dir.join("a.ifc");
        let error = publish_file(&target, false, |_| {
            Err(OutputError::new(ErrorCode::LimitExceeded, "too big"))
        })
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::LimitExceeded);
        assert!(leftovers(&dir).is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_directory_and_a_directory_target_are_refused() {
        let dir = scratch("dirs");
        let missing = dir.join("nope").join("a.ifc");
        assert_eq!(
            publish_file(&missing, false, put("x")).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        let as_dir = dir.join("sub");
        fs::create_dir(&as_dir).unwrap();
        assert_eq!(
            publish_file(&as_dir, true, put("x")).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            publish_file(Path::new(""), false, put("x"))
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_link_at_the_name_or_the_parent_is_refused() {
        let dir = scratch("links");
        let real = dir.join("real.ifc");
        fs::write(&real, "keep").unwrap();
        let link = dir.join("link.ifc");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        for overwrite in [false, true] {
            let error = publish_file(&link, overwrite, put("x")).unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidArgument);
        }
        assert_eq!(fs::read_to_string(&real).unwrap(), "keep");
        let sub = dir.join("sub");
        fs::create_dir(&sub).unwrap();
        let dir_link = dir.join("dirlink");
        std::os::unix::fs::symlink(&sub, &dir_link).unwrap();
        let error = publish_file(&dir_link.join("a.ifc"), false, put("x")).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidArgument);
        assert!(leftovers(&sub).is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn network_and_device_paths_are_recognised_as_text() {
        for network in [
            r"\\server\share\a.ifc",
            "//server/share/a.ifc",
            r"\\?\UNC\server\share\a.ifc",
            r"\\.\COM1",
        ] {
            assert!(is_network_or_device_path(network), "{network}");
        }
        for local in [
            r"\\?\C:\out\a.ifc",
            r"C:\out\a.ifc",
            "/tmp/a.ifc",
            "a.ifc",
            r"out\a.ifc",
        ] {
            assert!(!is_network_or_device_path(local), "{local}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_link_anywhere_above_the_output_is_refused() {
        let dir = scratch("ancestor");
        let real = dir.join("real");
        fs::create_dir_all(real.join("child")).unwrap();
        std::os::unix::fs::symlink(&real, dir.join("alias")).unwrap();
        let through = dir.join("alias").join("child").join("a.ifc");
        let error = publish_file(&through, false, put("x")).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidArgument);
        assert!(leftovers(&real.join("child")).is_empty());
        // The same file reached by a path without a link is fine.
        publish_file(&real.join("child").join("a.ifc"), false, put("x")).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn nothing_is_installed_until_every_staged_output_is_written() {
        let dir = scratch("staged");
        let (a, b) = (dir.join("a.ifc"), dir.join("b.json"));
        let first = stage_file(&a, false, put("one")).unwrap();
        assert!(!a.exists(), "staging installs nothing");
        // A second output that cannot be staged leaves the first one uninstalled.
        fs::write(&b, "keep").unwrap();
        assert_eq!(
            stage_file(&b, false, put("two")).err().unwrap().code,
            ErrorCode::InvalidArgument
        );
        drop(first);
        assert!(leftovers(&dir).iter().all(|n| !n.contains(".tmp-")));
        assert!(!a.exists());
        // Installed outputs can be taken back when the next one fails.
        let installed = stage_file(&a, false, put("one"))
            .unwrap()
            .install()
            .unwrap();
        assert_eq!(fs::read_to_string(&a).unwrap(), "one");
        installed.undo();
        assert!(!a.exists());
        // A replaced file is not removed by undo.
        fs::write(&a, "old").unwrap();
        stage_file(&a, true, put("new"))
            .unwrap()
            .install()
            .unwrap()
            .undo();
        assert_eq!(fs::read_to_string(&a).unwrap(), "new");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn destinations_compare_by_directory_and_name() {
        let dir = scratch("same");
        fs::create_dir(dir.join("sub")).unwrap();
        let a = resolve_destination(&dir.join("a.ifc")).unwrap();
        let again = resolve_destination(&dir.join("sub").join("..").join("a.ifc")).unwrap();
        let other = resolve_destination(&dir.join("b.ifc")).unwrap();
        let elsewhere = resolve_destination(&dir.join("sub").join("a.ifc")).unwrap();
        assert!(a.same_file(&again));
        assert!(!a.same_file(&other) && !a.same_file(&elsewhere));
        fs::remove_dir_all(&dir).unwrap();
    }
}
