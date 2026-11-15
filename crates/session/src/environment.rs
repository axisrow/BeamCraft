// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The environments a session can start with.
//!
//! An environment is the A07.3 environment layer (`environment.usda`) in a folder of its own. It
//! declares an id, an optional title and an ordered list of libraries that sit beside it.
//! [`list_environments`] reads the folders of the built-in environments shipped with the program
//! and of the user's own ones, in that order; the first environment of an id wins and a later one
//! of the same id is reported as shadowed. An environment that does not read is listed with its
//! refusal, never skipped silently.
//!
//! [`parse_environment`] reads only the declarations of the layer. Loading the libraries it lists
//! into definitions is a command and stays with the command line.

use crate::foreign;
use crate::{Refusal, settings};
use beamcraft_layer_engine::Value;
use beamcraft_model::command::label_is_valid;
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// File name of an environment layer inside its folder.
pub const ENVIRONMENT_FILE: &str = "environment.usda";
/// Name of the folder of environments, next to the program and in the user's settings directory.
pub const ENVIRONMENTS_DIR: &str = "environments";
/// Name of the folder of the user's own libraries, in the user's settings directory beside its
/// [`ENVIRONMENTS_DIR`].
pub const LIBRARIES_DIR: &str = "libraries";
/// The id of the shipped neutral environment: the built-in fallback layer every lookup reaches
/// when no stronger layer declares a designation.
pub const NEUTRAL_ENVIRONMENT_ID: &str = "blank";
/// Largest environment file, in bytes.
pub const MAX_ENVIRONMENT_BYTES: usize = 1024 * 1024;
/// Most libraries one environment may list, per list (profile, fastener and material); more is
/// `LIMIT_EXCEEDED`.
pub const MAX_ENVIRONMENT_LIBRARIES: usize = 32;
/// Most directory entries [`list_environments`] looks at in one folder.
pub const MAX_SCANNED: usize = 4_096;
const MAX_LIBRARY_REFERENCE_BYTES: usize = 256;

/// What an environment file declares about itself.
#[derive(Debug, Clone, PartialEq)]
pub struct EnvironmentInfo {
    /// `bim:environment:id`.
    pub id: String,
    /// `bim:environment:title`, when declared.
    pub title: Option<String>,
    /// SHA-256 of the environment file bytes.
    pub digest: ContentDigest,
    /// Size of the environment file in bytes.
    pub bytes: u64,
    /// `bim:environment:libraries`, strongest first, exactly as written.
    pub libraries: Vec<String>,
    /// `bim:environment:fastener-libraries`, strongest first, exactly as written; empty when the
    /// environment names none.
    pub fastener_libraries: Vec<String>,
    /// `bim:environment:material-libraries`, strongest first, exactly as written; empty when the
    /// environment names none.
    pub material_libraries: Vec<String>,
    /// `bim:environment:presets`, the contained relative path of the part presets, when declared.
    pub presets: Option<String>,
}

const fn refuse(code: ErrorCode, message: &'static str) -> Refusal {
    Refusal::new(code, message)
}

const fn invalid(message: &'static str) -> Refusal {
    Refusal::invalid(message)
}

fn label(value: Option<&Value>, message: &'static str) -> Result<Option<String>, Refusal> {
    match value {
        None => Ok(None),
        Some(Value::Text(text)) if label_is_valid(text) => Ok(Some(text.clone())),
        Some(_) => Err(invalid(message)),
    }
}

/// Whether a library reference of an environment is a relative, `/`-separated local path that
/// cannot leave the environment's directory: no absolute path, drive, scheme, backslash, empty,
/// `.` or `..` segment.
fn contained_reference(reference: &str) -> bool {
    !reference.is_empty()
        && reference.len() <= MAX_LIBRARY_REFERENCE_BYTES
        && !reference.starts_with('/')
        && !reference
            .chars()
            .any(|c| c <= '\u{1f}' || c == '\u{7f}' || c == '\\' || c == ':')
        && reference
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

/// One `bim:environment:*-libraries` array: a non-empty ordered list of contained relative
/// paths, none of them twice (`seen` spans every list, so the same file cannot serve as a
/// profile, a fastener and a material library of one environment).
fn library_list(items: &[Value], seen: &mut Vec<String>) -> Result<Vec<String>, Refusal> {
    if items.is_empty() {
        return Err(invalid("the environment lists no libraries"));
    }
    if items.len() > MAX_ENVIRONMENT_LIBRARIES {
        return Err(refuse(
            ErrorCode::LimitExceeded,
            "the environment lists more than 32 libraries",
        ));
    }
    let mut references = Vec::with_capacity(items.len());
    for item in items {
        let Value::Text(reference) = item else {
            return Err(invalid("an environment library is not a string"));
        };
        if !contained_reference(reference) {
            return Err(refuse(
                ErrorCode::PermissionDenied,
                "an environment library is not a relative path inside the environment directory",
            ));
        }
        if seen.contains(reference) {
            return Err(invalid("the environment lists a library twice"));
        }
        seen.push(reference.clone());
        references.push(reference.clone());
    }
    Ok(references)
}

/// Reads the declarations of an environment layer without touching any library.
///
/// The environment is a USD layer whose `customLayerData` carries `bim:environment:id`, an
/// optional `bim:environment:title`, `bim:environment:libraries`, a non-empty ordered list of
/// contained relative profile-library paths, and optionally
/// `bim:environment:fastener-libraries` and `bim:environment:material-libraries`, the same kind
/// of list for the environment's fastener and material libraries; all are strongest first. A list
/// entry that is not contained is `PERMISSION_DENIED` and more than 32 libraries in one list is
/// `LIMIT_EXCEEDED`; anything else malformed is `INVALID_ARGUMENT`. (The layer engine does not
/// expose composition arcs, and composing the libraries as sublayers would merge entries instead
/// of letting the strongest library answer, so the order is an explicit list; see the A07 design
/// note.)
///
/// # Errors
/// A typed [`Refusal`] as described above.
pub fn parse_environment(bytes: Vec<u8>) -> Result<EnvironmentInfo, Refusal> {
    if bytes.len() > MAX_ENVIRONMENT_BYTES {
        return Err(refuse(
            ErrorCode::LimitExceeded,
            "the environment file is larger than 1 MiB",
        ));
    }
    let layer = foreign::open(
        bytes,
        "the environment is not a self-contained USD layer in millimetres, Z up",
    )?;
    let Some(Value::Dictionary(data)) = layer.opened.metadata.get("customLayerData") else {
        return Err(invalid("the environment declares no bim:environment:id"));
    };
    let get = |key: &str| {
        label(
            data.get(key),
            "a bim:environment value is not a valid string label",
        )
    };
    let id = get("bim:environment:id")?
        .ok_or_else(|| invalid("the environment declares no bim:environment:id"))?;
    let title = get("bim:environment:title")?;
    let mut seen = Vec::new();
    let libraries = match data.get("bim:environment:libraries") {
        Some(Value::Array(items)) => library_list(items, &mut seen)?,
        _ => return Err(invalid("the environment lists no libraries")),
    };
    let fastener_libraries = match data.get("bim:environment:fastener-libraries") {
        None => Vec::new(),
        Some(Value::Array(items)) => library_list(items, &mut seen)?,
        Some(_) => {
            return Err(invalid(
                "the environment fastener libraries are not a string list",
            ));
        }
    };
    let material_libraries = match data.get("bim:environment:material-libraries") {
        None => Vec::new(),
        Some(Value::Array(items)) => library_list(items, &mut seen)?,
        Some(_) => {
            return Err(invalid(
                "the environment material libraries are not a string list",
            ));
        }
    };
    let presets = match data.get("bim:environment:presets") {
        None => None,
        Some(Value::Text(reference)) if contained_reference(reference) => Some(reference.clone()),
        Some(Value::Text(_)) => {
            return Err(refuse(
                ErrorCode::PermissionDenied,
                "the environment presets are not a relative path inside the environment directory",
            ));
        }
        Some(_) => return Err(invalid("the environment presets are not a string")),
    };
    Ok(EnvironmentInfo {
        id,
        title,
        digest: layer.digest,
        bytes: layer.bytes,
        libraries,
        fastener_libraries,
        material_libraries,
        presets,
    })
}

/// One environment found in an environments folder.
#[derive(Debug, Clone, PartialEq)]
pub struct EnvironmentEntry {
    /// The environment file.
    pub path: PathBuf,
    /// Where it was found: `builtin` or `user`.
    pub origin: &'static str,
    /// What reading it found.
    pub state: EnvironmentState,
}

/// The state of one environment file.
#[derive(Debug, Clone, PartialEq)]
pub enum EnvironmentState {
    /// A readable environment.
    Ready {
        /// `bim:environment:id`.
        id: String,
        /// `bim:environment:title`, when declared.
        title: Option<String>,
        /// Another environment of the same id was found first and answers for it.
        shadowed: bool,
    },
    /// An environment file that does not read.
    Refused(Refusal),
}

/// The folders of the built-in environments: `environments/` next to the program, inside the
/// macOS application bundle's `Resources`, and in the source tree of a development build.
#[must_use]
pub fn builtin_environment_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        dirs.push(dir.join(ENVIRONMENTS_DIR));
        dirs.push(dir.join("../Resources").join(ENVIRONMENTS_DIR));
    }
    dirs.push(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(ENVIRONMENTS_DIR),
    );
    dirs
}

/// The folders to read for the environments, built-in first and then the user's, each tagged with
/// its origin, and the user's folder on its own. The user's folder lies in `config_dir`, or in the
/// settings directory of the current user when `config_dir` is `None`.
#[must_use]
pub fn environment_folders(
    config_dir: Option<&Path>,
) -> (Vec<(PathBuf, &'static str)>, Option<PathBuf>) {
    let mut folders: Vec<(PathBuf, &'static str)> = builtin_environment_dirs()
        .into_iter()
        .map(|d| (d, "builtin"))
        .collect();
    let user = config_dir
        .map(Path::to_path_buf)
        .or_else(settings::default_dir)
        .map(|d| d.join(ENVIRONMENTS_DIR));
    if let Some(user) = &user {
        folders.push((user.clone(), "user"));
    }
    (folders, user)
}

/// The folder of the user's own libraries: `libraries/` under the application data directory
/// (`config_dir`, or the settings directory of the current user when `config_dir` is `None`),
/// beside its [`ENVIRONMENTS_DIR`]. The folder is where a person keeps library files of their
/// own; the programs never create or write it, and no environment update ever touches it — it
/// is the user-owned firm layer of the lookup.
#[must_use]
pub fn libraries_dir(config_dir: Option<&Path>) -> Option<PathBuf> {
    config_dir
        .map(Path::to_path_buf)
        .or_else(settings::default_dir)
        .map(|dir| dir.join(LIBRARIES_DIR))
}

/// The library files of the firm layer, in file-name order: every regular file of
/// [`libraries_dir`] whose name ends in `.usda` or `.usdc` (case-insensitive), at most
/// [`MAX_SCANNED`] of them. A missing or unreadable folder is no files; every other file there
/// (a note, a backup copy) is ignored by the scan and never reported.
#[must_use]
pub fn firm_library_files(config_dir: Option<&Path>) -> Vec<PathBuf> {
    let Some(dir) = libraries_dir(config_dir) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .take(MAX_SCANNED)
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("usda") || e.eq_ignore_ascii_case("usdc"))
        })
        .collect();
    files.sort();
    files
}

/// The file of the environment `id` among `folders`: the first ready one that no earlier
/// environment of the same id shadows, the answer [`list_environments`] reports for it. `None`
/// when no folder holds an environment of that id, or every one of that id is shadowed or
/// refused.
#[must_use]
pub fn find_environment(folders: &[(PathBuf, &'static str)], id: &str) -> Option<PathBuf> {
    list_environments(folders)
        .into_iter()
        .find_map(|entry| match entry.state {
            EnvironmentState::Ready {
                id: ready,
                shadowed: false,
                ..
            } if ready == id => Some(entry.path),
            _ => None,
        })
}

/// The environments of the given folders (each `(folder, origin)`), in folder order and by folder
/// name inside one folder. Missing folders are skipped; the same environment file reached twice
/// (a development build's folder is also the program's) is listed once.
#[must_use]
pub fn list_environments(folders: &[(PathBuf, &'static str)]) -> Vec<EnvironmentEntry> {
    let mut found: Vec<EnvironmentEntry> = Vec::new();
    let mut seen_files: Vec<PathBuf> = Vec::new();
    for (folder, origin) in folders {
        let Ok(entries) = fs::read_dir(folder) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .take(MAX_SCANNED)
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path().join(ENVIRONMENT_FILE))
            .filter(|f| f.is_file())
            .collect();
        files.sort();
        for file in files {
            // The canonical path only detects the same file reached twice. It is never reported:
            // on Windows it is a `\\?\` device path, which every file command refuses.
            let canonical = fs::canonicalize(&file).unwrap_or_else(|_| file.clone());
            if seen_files.contains(&canonical) {
                continue;
            }
            seen_files.push(canonical);
            let file = plain(&file);
            let state = match read_environment(&file) {
                Ok((id, title)) => {
                    let shadowed = found.iter().any(|e| {
                        matches!(&e.state, EnvironmentState::Ready { id: other, .. } if *other == id)
                    });
                    EnvironmentState::Ready {
                        id,
                        title,
                        shadowed,
                    }
                }
                Err(error) => EnvironmentState::Refused(error),
            };
            found.push(EnvironmentEntry {
                path: file,
                origin,
                state,
            });
        }
    }
    found
}

/// `path` made absolute with its `.` and `..` segments folded away, on the text alone: the form
/// a person and every file command accept (no `\\?\` prefix, no link resolution).
fn plain(path: &Path) -> PathBuf {
    use std::path::Component;
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn read_environment(file: &Path) -> Result<(String, Option<String>), Refusal> {
    let mut bytes = Vec::new();
    fs::File::open(file)
        .and_then(|f| {
            f.take(u64::try_from(MAX_ENVIRONMENT_BYTES).unwrap_or(u64::MAX) + 1)
                .read_to_end(&mut bytes)
        })
        .map_err(|_| {
            refuse(
                ErrorCode::NotFound,
                "the environment file could not be read",
            )
        })?;
    let info = parse_environment(bytes)?;
    Ok((info.id, info.title))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

    use super::*;

    fn layer(id: &str, libraries: &str) -> String {
        format!(
            "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {{\n        string \"bim:environment:id\" = \"{id}\"\n        string[] \"bim:environment:libraries\" = {libraries}\n    }}\n)\n"
        )
    }

    /// The header of a layer whose `customLayerData` is still open, with the required profile
    /// list already in place; entries append before the closing braces.
    fn layer_head(id: &str) -> String {
        layer(id, "[\"a.usda\"]").replace("    }\n)\n", "")
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("session-environment-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(root: &Path, folder: &str, text: &str) {
        let dir = root.join(folder);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(ENVIRONMENT_FILE), text).unwrap();
    }

    #[test]
    fn reported_paths_are_plain() {
        let p = plain(Path::new("/a/b/../c/./d.usda"));
        // On Windows the root gains the current drive; the rest is the same everywhere.
        assert!(p.is_absolute() && p.ends_with("a/c/d.usda"), "{p:?}");
        assert!(p.components().all(|c| !matches!(
            c,
            std::path::Component::ParentDir | std::path::Component::CurDir
        )));
        assert!(!p.to_string_lossy().starts_with("\\\\"));
    }

    #[test]
    fn an_environment_declares_its_id_and_libraries() {
        let info =
            parse_environment(layer("steel", "[\"a.usda\", \"sub/b.usda\"]").into_bytes()).unwrap();
        assert_eq!(info.id, "steel");
        assert_eq!(info.libraries, ["a.usda", "sub/b.usda"]);
        assert_eq!(info.title, None);
    }

    #[test]
    fn an_environment_may_declare_fastener_libraries() {
        let declared = format!(
            "{}        string[] \"bim:environment:fastener-libraries\" = [\"bolts.usda\", \"sub/nuts.usda\"]\n    }}\n)\n",
            layer_head("e")
        );
        let info = parse_environment(declared.into_bytes()).unwrap();
        assert_eq!(info.fastener_libraries, ["bolts.usda", "sub/nuts.usda"]);
        assert_eq!(info.libraries, ["a.usda"]);
        let none = parse_environment(layer("e", "[\"a.usda\"]").into_bytes()).unwrap();
        assert_eq!(none.fastener_libraries, Vec::<String>::new());
        // The same containment rules as the profile list.
        let escaped = parse_environment(
            format!(
                "{}        string[] \"bim:environment:fastener-libraries\" = [\"../b.usda\"]\n    }}\n)\n",
                layer_head("e")
            )
            .into_bytes(),
        )
        .unwrap_err();
        assert_eq!(escaped.code, ErrorCode::PermissionDenied);
        // The same file cannot serve as a profile and a fastener library of one environment.
        let twice = parse_environment(
            format!(
                "{}        string[] \"bim:environment:fastener-libraries\" = [\"a.usda\"]\n    }}\n)\n",
                layer_head("e")
            )
            .into_bytes(),
        )
        .unwrap_err();
        assert_eq!(twice.code, ErrorCode::InvalidArgument);
        // An empty list is no list.
        let empty = parse_environment(
            format!(
                "{}        string[] \"bim:environment:fastener-libraries\" = []\n    }}\n)\n",
                layer_head("e")
            )
            .into_bytes(),
        )
        .unwrap_err();
        assert_eq!(empty.code, ErrorCode::InvalidArgument);
        // A non-string entry is refused for the material list too, and the same file cannot
        // serve as a profile and a material library of one environment.
        let mistyped = parse_environment(
            format!(
                "{}        string \"bim:environment:material-libraries\" = \"a.usda\"\n    }}\n)\n",
                layer_head("e")
            )
            .into_bytes(),
        )
        .unwrap_err();
        assert_eq!(mistyped.code, ErrorCode::InvalidArgument);
        let twice = parse_environment(
            format!(
                "{}        string[] \"bim:environment:material-libraries\" = [\"a.usda\"]\n    }}\n)\n",
                layer_head("e")
            )
            .into_bytes(),
        )
        .unwrap_err();
        assert_eq!(twice.code, ErrorCode::InvalidArgument);
    }

    #[test]
    fn an_environment_may_declare_material_libraries() {
        let declared = format!(
            "{}        string[] \"bim:environment:material-libraries\" = [\"grades.usda\", \"sub/more.usda\"]\n    }}\n)\n",
            layer_head("e")
        );
        let info = parse_environment(declared.into_bytes()).unwrap();
        assert_eq!(info.material_libraries, ["grades.usda", "sub/more.usda"]);
        assert_eq!(info.libraries, ["a.usda"]);
        let none = parse_environment(layer("e", "[\"a.usda\"]").into_bytes()).unwrap();
        assert_eq!(none.material_libraries, Vec::<String>::new());
        // The same containment rules as the other lists.
        let escaped = parse_environment(
            format!(
                "{}        string[] \"bim:environment:material-libraries\" = [\"../m.usda\"]\n    }}\n)\n",
                layer_head("e")
            )
            .into_bytes(),
        )
        .unwrap_err();
        assert_eq!(escaped.code, ErrorCode::PermissionDenied);
        // One file cannot serve as a fastener and a material library of one environment.
        let shared = parse_environment(
            format!(
                "{}        string[] \"bim:environment:fastener-libraries\" = [\"g.usda\"]\n        string[] \"bim:environment:material-libraries\" = [\"g.usda\"]\n    }}\n)\n",
                layer_head("e")
            )
            .into_bytes(),
        )
        .unwrap_err();
        assert_eq!(shared.code, ErrorCode::InvalidArgument);
    }

    #[test]
    fn an_environment_may_declare_contained_presets() {
        let declared = |reference: &str| {
            format!(
                "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {{\n        string \"bim:environment:id\" = \"e\"\n        string[] \"bim:environment:libraries\" = [\"a.usda\"]\n        string \"bim:environment:presets\" = \"{reference}\"\n    }}\n)\n"
            )
        };
        let info = parse_environment(declared("presets.usda").into_bytes()).unwrap();
        assert_eq!(info.presets.as_deref(), Some("presets.usda"));
        let none = parse_environment(layer("e", "[\"a.usda\"]").into_bytes()).unwrap();
        assert_eq!(none.presets, None);
        let escaped = parse_environment(declared("../presets.usda").into_bytes()).unwrap_err();
        assert_eq!(escaped.code, ErrorCode::PermissionDenied);
    }

    #[test]
    fn an_uncontained_or_repeated_library_is_refused() {
        for bad in [
            "[\"../x.usda\"]",
            "[\"/x.usda\"]",
            "[\"C:/x.usda\"]",
            "[\"a\\\\b\"]",
        ] {
            let error = parse_environment(layer("e", bad).into_bytes()).unwrap_err();
            assert_eq!(error.code, ErrorCode::PermissionDenied, "{bad}");
        }
        let twice = parse_environment(layer("e", "[\"a.usda\", \"a.usda\"]").into_bytes());
        assert_eq!(twice.unwrap_err().code, ErrorCode::InvalidArgument);
        let none = parse_environment(layer("e", "[]").into_bytes());
        assert_eq!(none.unwrap_err().code, ErrorCode::InvalidArgument);
    }

    #[test]
    fn an_environment_reference_cannot_leave_its_directory() {
        for good in ["a.usda", "libs/a.usda", "a b/c.usda"] {
            assert!(contained_reference(good), "{good}");
        }
        for bad in [
            "",
            "/a.usda",
            "../a.usda",
            "libs/../a.usda",
            "./a.usda",
            "libs//a.usda",
            "libs/",
            "C:/a.usda",
            "a\\b.usda",
            "file:///a.usda",
            "a\u{0}.usda",
        ] {
            assert!(!contained_reference(bad), "{bad:?}");
        }
        assert!(!contained_reference(&"a".repeat(257)));
    }

    #[test]
    fn too_many_libraries_and_non_usd_bytes_are_refused() {
        let many: Vec<String> = (0..=MAX_ENVIRONMENT_LIBRARIES)
            .map(|n| format!("\"l{n}.usda\""))
            .collect();
        let error = parse_environment(layer("e", &format!("[{}]", many.join(", "))).into_bytes())
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::LimitExceeded);
        assert_eq!(
            parse_environment(b"not usd".to_vec()).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn a_file_over_the_limit_is_refused_before_it_is_read() {
        let error = parse_environment(vec![b' '; MAX_ENVIRONMENT_BYTES + 1]).unwrap_err();
        assert_eq!(error.code, ErrorCode::LimitExceeded);
    }

    #[test]
    fn listing_marks_the_first_of_an_id_and_reports_a_refused_file() {
        let builtin = scratch("builtin");
        let user = scratch("user");
        put(&builtin, "one", &layer("shared", "[\"p.usda\"]"));
        put(&user, "again", &layer("shared", "[\"p.usda\"]"));
        put(&user, "broken", "not a layer");
        let entries = list_environments(&[(builtin.clone(), "builtin"), (user.clone(), "user")]);
        assert_eq!(entries.len(), 3);
        let shadowed: Vec<_> = entries
            .iter()
            .filter_map(|e| match &e.state {
                EnvironmentState::Ready { shadowed, .. } => Some((e.origin, *shadowed)),
                EnvironmentState::Refused(_) => None,
            })
            .collect();
        assert_eq!(shadowed, [("builtin", false), ("user", true)]);
        assert_eq!(
            entries
                .iter()
                .filter(|e| matches!(e.state, EnvironmentState::Refused(_)))
                .count(),
            1
        );
        let _ = fs::remove_dir_all(&builtin);
        let _ = fs::remove_dir_all(&user);
    }

    #[test]
    fn the_libraries_folder_sits_beside_the_environments_folder() {
        let dir = scratch("libraries");
        assert_eq!(
            libraries_dir(Some(&dir)),
            Some(dir.join(LIBRARIES_DIR)),
            "libraries/ under the given application data directory"
        );
        assert_eq!(
            libraries_dir(None),
            settings::default_dir().map(|d| d.join(LIBRARIES_DIR))
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn firm_library_files_are_sorted_library_files_only() {
        let dir = scratch("firm-files");
        let libraries = dir.join(LIBRARIES_DIR);
        fs::create_dir_all(libraries.join("nested")).unwrap();
        fs::write(libraries.join("b.usda"), "x").unwrap();
        fs::write(libraries.join("a.USDA"), "x").unwrap();
        fs::write(libraries.join("c.usdc"), "x").unwrap();
        fs::write(libraries.join("notes.txt"), "x").unwrap();
        fs::write(libraries.join("d.partial.usda"), "x").unwrap();
        fs::write(libraries.join("nested/e.usda"), "x").unwrap();
        let found = firm_library_files(Some(&dir));
        let names: Vec<String> = found
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        assert_eq!(
            names,
            ["a.USDA", "b.usda", "c.usdc", "d.partial.usda"],
            "sorted by name, library extensions only, no subfolder descent"
        );
        assert!(found.iter().all(|p| p.starts_with(&libraries)));
        // A missing folder is no files, and the scan never creates anything.
        assert_eq!(
            firm_library_files(Some(&dir.join("absent"))),
            Vec::<PathBuf>::new()
        );
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            1,
            "only libraries/ was made by the test"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_environment_answers_the_first_ready_one_of_its_id() {
        let builtin = scratch("find-builtin");
        let user = scratch("find-user");
        put(&builtin, "one", &layer("shared", "[\"p.usda\"]"));
        put(&user, "again", &layer("shared", "[\"p.usda\"]"));
        put(&user, "other", &layer("other", "[\"p.usda\"]"));
        put(&user, "broken", "not a layer");
        let folders = [(builtin.clone(), "builtin"), (user.clone(), "user")];
        let shared = find_environment(&folders, "shared");
        assert!(
            shared
                .as_ref()
                .is_some_and(|p| p.ends_with("one/environment.usda")),
            "the first folder's environment answers: {shared:?}"
        );
        let other = find_environment(&folders, "other");
        assert!(
            other
                .as_ref()
                .is_some_and(|p| p.ends_with("other/environment.usda")),
            "a later folder's own id is found: {other:?}"
        );
        assert!(shared.is_some_and(|p| p.is_file()) && other.is_some_and(|p| p.is_file()));
        assert_eq!(
            find_environment(&folders, "missing"),
            None,
            "no folder holds that id"
        );
        let user_only = [(user.clone(), "user")];
        assert!(
            find_environment(&user_only, "shared")
                .is_some_and(|p| p.ends_with("again/environment.usda")),
            "with the first folder gone the user's copy answers"
        );
        let _ = fs::remove_dir_all(&builtin);
        let _ = fs::remove_dir_all(&user);
    }

    #[test]
    fn the_shipped_environments_read() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(ENVIRONMENTS_DIR);
        let entries = list_environments(&[(root, "builtin")]);
        assert!(
            entries
                .iter()
                .all(|e| matches!(e.state, EnvironmentState::Ready { .. })),
            "{entries:?}"
        );
        assert!(
            entries.iter().any(|e| matches!(
                &e.state,
                EnvironmentState::Ready { id, .. } if id == "blank"
            )),
            "the base blank environment ships with the program"
        );
        assert!(
            entries.iter().any(|e| matches!(
                &e.state,
                EnvironmentState::Ready { id, .. } if id == "europe"
            )),
            "the shipped regional europe environment reads"
        );
        assert!(
            entries.iter().any(|e| matches!(
                &e.state,
                EnvironmentState::Ready { id, .. } if id == "russia"
            )),
            "the shipped regional russia environment reads"
        );
        assert!(
            entries.iter().any(|e| matches!(
                &e.state,
                EnvironmentState::Ready { id, .. } if id == "australia"
            )),
            "the shipped regional australia environment reads"
        );
        assert!(
            entries.iter().any(|e| matches!(
                &e.state,
                EnvironmentState::Ready { id, .. } if id == "india"
            )),
            "the shipped regional india environment reads"
        );
        assert!(
            entries.iter().any(|e| matches!(
                &e.state,
                EnvironmentState::Ready { id, .. } if id == "china"
            )),
            "the shipped regional china environment reads"
        );
    }
}
