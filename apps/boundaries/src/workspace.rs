// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Loads the workspace members, their manifests and their Rust sources from disk.
//!
//! Only `src/` is scanned for sources. A member's explicit target paths and whether it has a
//! build script are loaded too, so the checker can refuse a layout that would hide sources from
//! that scan (#208).
//!
//! Every read is bounded: manifests and sources are at most 1 MiB, at most 256 members and 512
//! source files per member are read and directories are walked at most eight levels deep.
//! The loader fails closed: a symbolic link or Windows reparse point anywhere in a member path, a
//! manifest or a source tree, a source directory deeper than the cap and a non-regular manifest
//! are errors, never silently skipped.

use std::fmt;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use crate::manifest::{
    Dependency, MAX_MANIFEST_BYTES, ManifestError, parse_manifest, parse_manifest_in,
};

/// Most workspace members read.
pub const MAX_MEMBERS: usize = 256;
/// Most source files read per member.
pub const MAX_SOURCE_FILES: usize = 512;
/// Deepest source directory level walked.
pub const MAX_DEPTH: usize = 8;

/// One Rust source file of a package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// Path relative to the workspace root, with `/` separators.
    pub path: String,
    /// File text.
    pub text: String,
}

/// One workspace member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    /// Cargo package name.
    pub name: String,
    /// Directory relative to the workspace root, with `/` separators.
    pub path: String,
    /// `publish = false`.
    pub publish_false: bool,
    /// Declared dependencies of every table.
    pub dependencies: Vec<Dependency>,
    /// Rust sources under `src/`.
    pub sources: Vec<Source>,
    /// Explicit target paths as `(table, path)` from the manifest (`[lib] path`, `[[bin]] path`,
    /// ...), relative to the package directory.
    pub targets: Vec<(String, String)>,
    /// Whether the package has a build script: a `build.rs` next to the manifest or a
    /// `[package] build` other than `false`.
    pub build_script: bool,
}

/// All members of a workspace.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Workspace {
    /// Members in path order.
    pub packages: Vec<Package>,
}

/// Why the workspace was not loaded.
#[derive(Debug)]
pub enum LoadError {
    /// A file or directory could not be read.
    Io {
        /// Path relative to the root.
        path: String,
        /// The operating-system error.
        source: std::io::Error,
    },
    /// A file is larger than the limit.
    TooLarge(String),
    /// A manifest exceeded a parser limit.
    Manifest {
        /// Path relative to the root.
        path: String,
        /// The parser error.
        error: ManifestError,
    },
    /// A manifest has no `[package] name`.
    NoName(String),
    /// A members pattern is not `dir/name` or `dir/*`, or leaves the workspace.
    BadMember(String),
    /// More members or source files than the limits allow.
    TooMany(String),
    /// A member, manifest or source entry is a symbolic link or reparse point.
    Link(String),
    /// A source directory is nested deeper than [`MAX_DEPTH`]; unscanned files would pass.
    TooDeep(String),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "cannot read `{path}`: {source}"),
            Self::TooLarge(path) => write!(f, "`{path}` is larger than the 1 MiB limit"),
            Self::Manifest { path, error } => write!(f, "`{path}`: {error}"),
            Self::NoName(path) => write!(f, "`{path}` has no [package] name"),
            Self::BadMember(pattern) => write!(f, "unsupported workspace member `{pattern}`"),
            Self::TooMany(what) => write!(f, "too many {what}"),
            Self::Link(path) => write!(
                f,
                "`{path}` is a symbolic link or reparse point; links are not followed"
            ),
            Self::TooDeep(path) => write!(
                f,
                "`{path}` is nested deeper than {MAX_DEPTH} levels and would not be scanned"
            ),
        }
    }
}

impl std::error::Error for LoadError {}

/// True for a symbolic link, and on Windows for any reparse point (junctions, mount points).
fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        const REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & REPARSE_POINT != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

/// What a path is on disk, without following a link.
#[derive(Debug, PartialEq, Eq)]
enum Kind {
    Missing,
    Dir,
    File,
    Other,
}

/// Classifies `rel` below `root`. A link at any component of `rel` is an error.
fn classify(root: &Path, rel: &str) -> Result<Kind, LoadError> {
    let mut current = PathBuf::from(root);
    let mut walked = String::new();
    let mut kind = Kind::Dir;
    for segment in rel.split('/') {
        if !walked.is_empty() {
            walked.push('/');
        }
        walked.push_str(segment);
        current.push(segment);
        kind = match fs::symlink_metadata(&current) {
            Ok(metadata) if is_link(&metadata) => return Err(LoadError::Link(walked)),
            Ok(metadata) if metadata.is_dir() => Kind::Dir,
            Ok(metadata) if metadata.is_file() => Kind::File,
            Ok(_) => Kind::Other,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Kind::Missing),
            Err(source) => {
                return Err(LoadError::Io {
                    path: walked,
                    source,
                });
            }
        };
    }
    Ok(kind)
}

fn read_bounded(root: &Path, rel: &str) -> Result<String, LoadError> {
    let io = |source| LoadError::Io {
        path: rel.to_owned(),
        source,
    };
    if classify(root, rel)? != Kind::File {
        return Err(io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "not a regular file",
        )));
    }
    let file = fs::File::open(root.join(rel)).map_err(io)?;
    let mut text = String::new();
    let limit = u64::try_from(MAX_MANIFEST_BYTES)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    file.take(limit).read_to_string(&mut text).map_err(io)?;
    if text.len() > MAX_MANIFEST_BYTES {
        return Err(LoadError::TooLarge(rel.to_owned()));
    }
    Ok(text)
}

fn valid_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && !segment.contains(['\\', ':', '*', '?'])
}

/// Expands one `members` entry: a literal `a/b` or a one-level glob `a/*`.
fn expand(root: &Path, pattern: &str) -> Result<Vec<String>, LoadError> {
    let bad = || LoadError::BadMember(pattern.to_owned());
    if let Some(dir) = pattern.strip_suffix("/*") {
        if dir.is_empty() || !dir.split('/').all(valid_segment) {
            return Err(bad());
        }
        let io = |source| LoadError::Io {
            path: dir.to_owned(),
            source,
        };
        if classify(root, dir)? != Kind::Dir {
            return Err(bad());
        }
        let mut found = Vec::new();
        for item in fs::read_dir(root.join(dir)).map_err(io)? {
            let item = item.map_err(io)?;
            let name = item.file_name().to_string_lossy().into_owned();
            if !valid_segment(&name) {
                continue;
            }
            // `classify` fails on a link at the entry itself or at its manifest.
            let member = format!("{dir}/{name}");
            if classify(root, &member)? == Kind::Dir
                && classify(root, &format!("{member}/Cargo.toml"))? == Kind::File
            {
                found.push(member);
            }
        }
        found.sort();
        Ok(found)
    } else if pattern.split('/').all(valid_segment) {
        // A literal member is checked for links when its manifest is read.
        Ok(vec![pattern.to_owned()])
    } else {
        Err(bad())
    }
}

fn collect_sources(
    root: &Path,
    rel: &str,
    depth: usize,
    out: &mut Vec<Source>,
) -> Result<(), LoadError> {
    let io = |source| LoadError::Io {
        path: rel.to_owned(),
        source,
    };
    let mut entries: Vec<(String, bool, bool)> = Vec::new();
    for entry in fs::read_dir(root.join(rel)).map_err(io)? {
        let entry = entry.map_err(io)?;
        let kind = entry.file_type().map_err(io)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // Any link is an error, whatever it points to: nothing is skipped silently.
        if kind.is_symlink() || is_link(&entry.metadata().map_err(io)?) {
            return Err(LoadError::Link(format!("{rel}/{name}")));
        }
        entries.push((name, kind.is_dir(), kind.is_file()));
    }
    entries.sort();
    for (name, is_dir, is_file) in entries {
        let child = format!("{rel}/{name}");
        if is_dir {
            if depth >= MAX_DEPTH {
                return Err(LoadError::TooDeep(child));
            }
            collect_sources(root, &child, depth.saturating_add(1), out)?;
        } else if is_file && name.ends_with(".rs") {
            if out.len() >= MAX_SOURCE_FILES {
                return Err(LoadError::TooMany("source files".to_owned()));
            }
            let text = read_bounded(root, &child)?;
            out.push(Source { path: child, text });
        }
    }
    Ok(())
}

/// Loads the workspace rooted at `root` (the directory with the workspace `Cargo.toml`).
///
/// # Errors
///
/// [`LoadError`] for unreadable, oversized or unsupported input.
pub fn load(root: &Path) -> Result<Workspace, LoadError> {
    let root_text = read_bounded(root, "Cargo.toml")?;
    let root_manifest = parse_manifest(&root_text).map_err(|error| LoadError::Manifest {
        path: "Cargo.toml".to_owned(),
        error,
    })?;
    let mut paths = Vec::new();
    for pattern in &root_manifest.members {
        paths.extend(expand(root, pattern)?);
    }
    if paths.len() > MAX_MEMBERS {
        return Err(LoadError::TooMany("workspace members".to_owned()));
    }
    paths.sort();
    paths.dedup();
    let mut packages = Vec::new();
    for path in paths {
        let manifest_path = format!("{path}/Cargo.toml");
        let text = read_bounded(root, &manifest_path)?;
        let manifest =
            parse_manifest_in(&text, &root_manifest.workspace_dependencies).map_err(|error| {
                LoadError::Manifest {
                    path: manifest_path.clone(),
                    error,
                }
            })?;
        let name = manifest.name.ok_or(LoadError::NoName(manifest_path))?;
        let mut sources = Vec::new();
        let src = format!("{path}/src");
        if classify(root, &src)? == Kind::Dir {
            collect_sources(root, &src, 0, &mut sources)?;
        }
        let build_script = match manifest.build.as_deref() {
            Some("false") => false,
            Some(_) => true,
            None => classify(root, &format!("{path}/build.rs"))? != Kind::Missing,
        };
        packages.push(Package {
            name,
            path,
            publish_false: manifest.publish_false,
            dependencies: manifest.dependencies,
            sources,
            targets: manifest.targets,
            build_script,
        });
    }
    Ok(Workspace { packages })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn members_must_stay_inside_the_workspace() {
        let root = Path::new(".");
        for bad in [
            "../x", "/abs", "a/../b", "a\\b", "C:/x", "*", "a/*/b", "/*", "*/x", "",
        ] {
            assert!(
                matches!(expand(root, bad), Err(LoadError::BadMember(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_literal_member_is_returned_as_is() {
        assert_eq!(
            expand(Path::new("."), "tools/fixtures").unwrap(),
            ["tools/fixtures"]
        );
    }

    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A scratch directory below the system temp directory, removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "beamcraft-boundaries-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn write(&self, rel: &str, text: &str) {
            let path = self.0.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const MANIFEST: &str = "[package]
name = \"x\"
";

    /// Creates a directory link (symlink, or a junction on Windows). False if not permitted.
    fn dir_link(link: &Path, target: &Path) -> bool {
        #[cfg(unix)]
        let ok = std::os::unix::fs::symlink(target, link).is_ok();
        #[cfg(windows)]
        let ok = std::os::windows::fs::symlink_dir(target, link).is_ok()
            || std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
        #[cfg(not(any(unix, windows)))]
        let ok = false;
        if !ok {
            eprintln!("skipped: this environment does not permit creating directory links");
        }
        ok
    }

    /// The #208 reproduction on disk: a member whose library lives in `code/` behind
    /// `[lib] path` (so `src/` is empty) and a member with a `build.rs`. The loader reports both
    /// facts, so the checker can refuse them instead of passing an unscanned crate.
    #[test]
    fn the_loader_reports_moved_targets_and_build_scripts() {
        let root = Scratch::new();
        root.write("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n");
        root.write(
            "crates/moved/Cargo.toml",
            "[package]\nname = \"moved\"\n[lib]\npath = \"code/lib.rs\"\n",
        );
        root.write(
            "crates/moved/code/lib.rs",
            "pub fn f() { let _ = std::fs::metadata(\".\"); }\n",
        );
        root.write("crates/built/Cargo.toml", "[package]\nname = \"built\"\n");
        root.write("crates/built/build.rs", "fn main() {}\n");
        root.write("crates/built/src/lib.rs", "\n");
        root.write(
            "crates/off/Cargo.toml",
            "[package]\nname = \"off\"\nbuild = false\n",
        );
        root.write("crates/off/build.rs", "fn main() {}\n");
        let ws = load(&root.0).unwrap();
        let get = |n: &str| ws.packages.iter().find(|p| p.name == n).unwrap();
        assert_eq!(
            get("moved").targets,
            [("lib".to_owned(), "code/lib.rs".to_owned())]
        );
        assert!(get("moved").sources.is_empty(), "nothing under src/");
        assert!(!get("moved").build_script);
        assert!(get("built").build_script);
        assert!(!get("off").build_script);
    }

    #[test]
    fn a_member_that_is_a_link_to_a_directory_outside_the_root_is_rejected() {
        let outside = Scratch::new();
        outside.write("Cargo.toml", MANIFEST);
        outside.write(
            "src/lib.rs",
            "pub fn f() {}
",
        );
        let root = Scratch::new();
        root.write(
            "Cargo.toml",
            "[workspace]
members = [\"crates/*\"]
",
        );
        fs::create_dir_all(root.0.join("crates")).unwrap();
        if !dir_link(&root.0.join("crates/evil"), &outside.0) {
            return;
        }
        let err = load(&root.0).unwrap_err();
        assert!(
            matches!(&err, LoadError::Link(p) if p == "crates/evil"),
            "{err}"
        );
    }

    #[test]
    fn a_literal_member_below_a_link_is_rejected() {
        let outside = Scratch::new();
        outside.write("m/Cargo.toml", MANIFEST);
        let root = Scratch::new();
        root.write(
            "Cargo.toml",
            "[workspace]
members = [\"linked/m\"]
",
        );
        if !dir_link(&root.0.join("linked"), &outside.0) {
            return;
        }
        let err = load(&root.0).unwrap_err();
        assert!(matches!(&err, LoadError::Link(p) if p == "linked"), "{err}");
    }

    #[test]
    fn a_link_inside_src_is_rejected_not_skipped() {
        let outside = Scratch::new();
        outside.write(
            "lib.rs",
            "use std::fs;
",
        );
        let root = Scratch::new();
        root.write(
            "Cargo.toml",
            "[workspace]
members = [\"crates/a\"]
",
        );
        root.write("crates/a/Cargo.toml", MANIFEST);
        root.write(
            "crates/a/src/lib.rs",
            "
",
        );
        if !dir_link(&root.0.join("crates/a/src/hidden"), &outside.0) {
            return;
        }
        let err = load(&root.0).unwrap_err();
        assert!(matches!(err, LoadError::Link(_)), "{err}");
    }

    #[test]
    fn an_ordinary_tree_loads() {
        let root = Scratch::new();
        root.write(
            "Cargo.toml",
            "[workspace]
members = [\"crates/*\"]
",
        );
        root.write("crates/a/Cargo.toml", MANIFEST);
        root.write(
            "crates/a/src/lib.rs",
            "fn a() {}
",
        );
        root.write(
            "crates/a/src/deep/mod.rs",
            "fn b() {}
",
        );
        let ws = load(&root.0).unwrap();
        assert_eq!(ws.packages.len(), 1);
        assert_eq!(ws.packages.first().map(|p| p.sources.len()), Some(2));
    }

    #[test]
    fn a_source_tree_exactly_at_the_depth_cap_loads_and_one_deeper_fails() {
        let nested = |levels: usize| {
            let root = Scratch::new();
            root.write(
                "Cargo.toml",
                "[workspace]
members = [\"crates/a\"]
",
            );
            root.write("crates/a/Cargo.toml", MANIFEST);
            let dirs = vec!["d"; levels].join("/");
            let rel = if dirs.is_empty() {
                "crates/a/src/lib.rs".to_owned()
            } else {
                format!("crates/a/src/{dirs}/lib.rs")
            };
            root.write(
                &rel,
                "use std::fs;
",
            );
            let result = load(&root.0);
            (root, result)
        };
        let (_keep, ok) = nested(MAX_DEPTH);
        let ws = ok.unwrap();
        assert_eq!(ws.packages.first().map(|p| p.sources.len()), Some(1));
        let (_keep, deep) = nested(MAX_DEPTH + 1);
        let err = deep.unwrap_err();
        assert!(
            matches!(&err, LoadError::TooDeep(p) if p.ends_with("/d")),
            "{err}"
        );
    }

    #[test]
    fn a_manifest_that_is_not_a_regular_file_is_rejected() {
        let root = Scratch::new();
        root.write(
            "Cargo.toml",
            "[workspace]
members = [\"crates/a\"]
",
        );
        fs::create_dir_all(root.0.join("crates/a/Cargo.toml")).unwrap();
        assert!(matches!(load(&root.0), Err(LoadError::Io { .. })));
    }
}
