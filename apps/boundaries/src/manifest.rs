// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A small, bounded reader for the parts of `Cargo.toml` that the boundary rules need.
//!
//! It understands `[package]` (`name`, `publish`, `build`), `[workspace]` (`members`), the target
//! tables (`[lib]`, `[[bin]]`, `[[test]]`, `[[bench]]`, `[[example]]` and their `path`) and every
//! dependency
//! table form: `[dependencies]`, `[dev-dependencies]`, `[build-dependencies]`, their
//! `[target.<cfg>.*]` variants and the `[dependencies.<name>]` sub-table form. A dependency's
//! fields are accumulated by its key, whether they are written inline (`x = { package = "p" }`),
//! dotted (`x.package = "p"`, `x.version = "1"`) or in a sub-table, and the checked name is the
//! real package (`package = "..."`). An inherited entry (`workspace = true`) is resolved against
//! `[workspace.dependencies]` of the workspace root. It is not a general TOML parser; anything
//! else is ignored.

use std::fmt;

/// Largest manifest accepted, bytes.
pub const MAX_MANIFEST_BYTES: usize = 1 << 20;
/// Most lines read from one manifest.
pub const MAX_LINES: usize = 10_000;

/// One dependency edge declared by a manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    /// The package name (after any `package = "..."` rename).
    pub name: String,
    /// The table it was declared in, for example `dependencies` or `target.'cfg(unix)'.dev-dependencies`.
    pub section: String,
}

/// What the boundary rules need from one manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    /// `[package] name`, if the manifest has a package.
    pub name: Option<String>,
    /// `[package] publish = false`.
    pub publish_false: bool,
    /// `[workspace] members` patterns.
    pub members: Vec<String>,
    /// Declared dependencies of every table, in file order, by real package name.
    pub dependencies: Vec<Dependency>,
    /// `[workspace.dependencies]` as `(key, real package name)` pairs, in file order.
    pub workspace_dependencies: Vec<(String, String)>,
    /// Explicit target paths as `(table, path)`, for example `("lib", "code/lib.rs")`, in file
    /// order. A path that is not one quoted string is kept as written, so it is never mistaken
    /// for a valid one.
    pub targets: Vec<(String, String)>,
    /// `[package] build` as written (`"build.rs"`, `true` or `false`), if present.
    pub build: Option<String>,
}

/// The target tables whose `path` the layout rule checks.
pub const TARGET_TABLES: [&str; 5] = ["lib", "bin", "test", "bench", "example"];

/// Why a manifest was not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestError {
    /// The text is larger than [`MAX_MANIFEST_BYTES`].
    TooLarge,
    /// The text has more than [`MAX_LINES`] lines.
    TooManyLines,
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => f.write_str("manifest is larger than the 1 MiB limit"),
            Self::TooManyLines => f.write_str("manifest has more than 10000 lines"),
        }
    }
}

impl std::error::Error for ManifestError {}

/// Removes a trailing `#` comment that is not inside a string.
fn strip_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    for (i, c) in line.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), _) if q == c => quote = None,
            (None, '#') => return line.get(..i).unwrap_or(line),
            _ => {}
        }
    }
    line
}

/// Every double- or single-quoted string in `text`.
fn quoted_strings(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current: Option<(char, String)> = None;
    for c in text.chars() {
        match current.take() {
            None if c == '"' || c == '\'' => current = Some((c, String::new())),
            None => {}
            Some((q, done)) if q == c => out.push(done),
            Some((q, mut s)) => {
                s.push(c);
                current = Some((q, s));
            }
        }
    }
    out
}

/// The value of a single quoted string, such as `"x"`.
fn string_value(value: &str) -> Option<String> {
    let strings = quoted_strings(value);
    match strings.as_slice() {
        [one] if value.trim().len() == one.len().saturating_add(2) => Some(one.clone()),
        _ => None,
    }
}

/// `package = "real"` inside an inline table value, if present.
fn inline_package(value: &str) -> Option<String> {
    let at = value.find("package")?;
    let rest = value.get(at.checked_add("package".len())?..)?;
    let rest = rest.trim_start().strip_prefix('=')?;
    quoted_strings(rest).into_iter().next()
}

/// Splits a table header into dot-separated tokens, keeping quoted tokens whole.
fn header_tokens(header: &str) -> Vec<String> {
    let mut tokens = vec![String::new()];
    let mut quote: Option<char> = None;
    for c in header.chars() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), _) if q == c => quote = None,
            (None, '.') => tokens.push(String::new()),
            (_, _) => {
                if let Some(last) = tokens.last_mut() {
                    last.push(c);
                }
            }
        }
    }
    tokens.into_iter().map(|t| t.trim().to_owned()).collect()
}

fn is_dependency_table(token: &str) -> bool {
    matches!(
        token,
        "dependencies" | "dev-dependencies" | "build-dependencies"
    )
}

/// One dependency key with the fields collected so far.
struct Entry {
    key: String,
    section: String,
    package: Option<String>,
    inherited: bool,
}

/// How the current section relates to dependencies.
enum Section {
    Other,
    Package,
    Workspace,
    /// A table of `name = value` lines.
    Deps(String),
    /// A `[dependencies.name]` sub-table for one dependency (index into the entries).
    Dep(usize),
    /// `[workspace.dependencies]`.
    WorkspaceDeps,
    /// A target table such as `[lib]` or `[[bin]]`.
    Target(String),
    /// A `[workspace.dependencies.name]` sub-table (index into the workspace entries).
    WorkspaceDep(usize),
}

/// True if an inline table `value` sets `key = true`.
fn inline_flag(value: &str, key: &str) -> bool {
    value.match_indices(key).any(|(at, _)| {
        let before = value.get(..at).and_then(|s| s.chars().next_back());
        let word_start = before.is_none_or(|c| c.is_whitespace() || c == '{' || c == ',');
        let after = value.get(at.saturating_add(key.len())..).unwrap_or("");
        word_start
            && after
                .trim_start()
                .strip_prefix('=')
                .is_some_and(|rest| rest.trim_start().starts_with("true"))
    })
}

/// Index of the entry for `key` in `section`, created if absent.
fn entry_index(entries: &mut Vec<Entry>, section: &str, key: &str) -> usize {
    if let Some(at) = entries
        .iter()
        .position(|e| e.key == key && e.section == section)
    {
        return at;
    }
    entries.push(Entry {
        key: key.to_owned(),
        section: section.to_owned(),
        package: None,
        inherited: false,
    });
    entries.len().saturating_sub(1)
}

/// Applies one field (`package` or `workspace`) to an entry.
fn set_field(entry: Option<&mut Entry>, field: &str, value: &str) {
    let Some(entry) = entry else { return };
    match field {
        "package" => {
            if let Some(real) = string_value(value) {
                entry.package = Some(real);
            }
        }
        "workspace" => entry.inherited |= value == "true",
        _ => {}
    }
}

/// Records one `key = value` line of a dependency table: inline (`x = { ... }`) or dotted
/// (`x.package = "p"`). The first dotted token is the dependency key.
fn record(entries: &mut Vec<Entry>, section: &str, key: &str, value: &str) {
    let tokens = header_tokens(key);
    let Some(dep) = tokens.first().filter(|t| !t.is_empty()) else {
        return;
    };
    let at = entry_index(entries, section, dep);
    match tokens.as_slice() {
        [_] => {
            if let Some(entry) = entries.get_mut(at) {
                if let Some(real) = inline_package(value) {
                    entry.package = Some(real);
                }
                entry.inherited |= inline_flag(value, "workspace");
            }
        }
        [_, field] => set_field(entries.get_mut(at), field, value),
        _ => {}
    }
}

/// Reads the facts the boundary rules need from `text`.
///
/// # Errors
///
/// [`ManifestError`] if the text exceeds the size or line limits.
pub fn parse_manifest(text: &str) -> Result<Manifest, ManifestError> {
    parse_manifest_in(text, &[])
}

/// Like [`parse_manifest`], resolving inherited dependencies (`workspace = true`) against
/// `root`, the `[workspace.dependencies]` pairs of the workspace root manifest, and then against
/// the manifest's own `[workspace.dependencies]`. An inherited key defined nowhere keeps its key
/// as the name (Cargo itself rejects such a manifest).
///
/// # Errors
///
/// [`ManifestError`] if the text exceeds the size or line limits.
pub fn parse_manifest_in(text: &str, root: &[(String, String)]) -> Result<Manifest, ManifestError> {
    if text.len() > MAX_MANIFEST_BYTES {
        return Err(ManifestError::TooLarge);
    }
    if text.lines().take(MAX_LINES.saturating_add(1)).count() > MAX_LINES {
        return Err(ManifestError::TooManyLines);
    }
    let mut manifest = Manifest::default();
    let mut section = Section::Other;
    let mut entries: Vec<Entry> = Vec::new();
    let mut workspace_entries: Vec<Entry> = Vec::new();
    let mut lines = text.lines();
    while let Some(raw) = lines.next() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(header) = line.strip_prefix('[') {
            let header = header
                .trim_start_matches('[')
                .trim_end()
                .trim_end_matches(']');
            section = classify(header, &mut entries, &mut workspace_entries);
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        match &section {
            Section::Package => match key {
                "name" => manifest.name = string_value(value),
                "publish" => manifest.publish_false = value == "false",
                "build" => {
                    manifest.build = Some(string_value(value).unwrap_or_else(|| value.to_owned()));
                }
                _ => {}
            },
            Section::Target(table) if key == "path" => manifest.targets.push((
                table.clone(),
                string_value(value).unwrap_or_else(|| value.to_owned()),
            )),
            Section::Workspace if key == "members" => {
                let mut array = value.to_owned();
                while !array.contains(']') {
                    let Some(next) = lines.next() else { break };
                    array.push(' ');
                    array.push_str(strip_comment(next));
                }
                manifest.members = quoted_strings(&array);
            }
            Section::Deps(name) => record(&mut entries, name, key, value),
            Section::WorkspaceDeps => record(&mut workspace_entries, "", key, value),
            Section::Dep(at) => set_field(entries.get_mut(*at), key, value),
            Section::WorkspaceDep(at) => set_field(workspace_entries.get_mut(*at), key, value),
            _ => {}
        }
    }
    manifest.workspace_dependencies = workspace_entries
        .iter()
        .map(|e| {
            (
                e.key.clone(),
                e.package.clone().unwrap_or_else(|| e.key.clone()),
            )
        })
        .collect();
    for entry in entries {
        let name = match (entry.package, entry.inherited) {
            (Some(real), _) => real,
            (None, true) => root
                .iter()
                .chain(manifest.workspace_dependencies.iter())
                .find(|(key, _)| *key == entry.key)
                .map_or(entry.key, |(_, real)| real.clone()),
            (None, false) => entry.key,
        };
        push_unique(&mut manifest, name, &entry.section);
    }
    Ok(manifest)
}

fn push_unique(manifest: &mut Manifest, name: String, section: &str) {
    let exists = manifest
        .dependencies
        .iter()
        .any(|d| d.name == name && d.section == section);
    if !exists {
        manifest.dependencies.push(Dependency {
            name,
            section: section.to_owned(),
        });
    }
}

fn classify(header: &str, entries: &mut Vec<Entry>, workspace_entries: &mut Vec<Entry>) -> Section {
    let tokens = header_tokens(header);
    let strs: Vec<&str> = tokens.iter().map(String::as_str).collect();
    match strs.as_slice() {
        ["package"] => Section::Package,
        ["workspace"] => Section::Workspace,
        ["workspace", "dependencies"] => Section::WorkspaceDeps,
        ["workspace", "dependencies", name] => {
            Section::WorkspaceDep(entry_index(workspace_entries, "", name))
        }
        ["workspace", ..] => Section::Other,
        [table] if TARGET_TABLES.contains(table) => Section::Target((*table).to_owned()),
        [.., last] if is_dependency_table(last) => Section::Deps(header.to_owned()),
        [.., table, name] if is_dependency_table(table) => {
            let section = tokens
                .iter()
                .take(tokens.len().saturating_sub(1))
                .cloned()
                .collect::<Vec<_>>()
                .join(".");
            Section::Dep(entry_index(entries, &section, name))
        }
        _ => Section::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deps(m: &Manifest) -> Vec<(&str, &str)> {
        m.dependencies
            .iter()
            .map(|d| (d.name.as_str(), d.section.as_str()))
            .collect()
    }

    #[test]
    fn reads_package_and_inherited_dependencies() {
        let m = parse_manifest(
            "[package]\nname = \"beamcraft-model\" # c\npublish = false\n\n\
             [dependencies]\nbeamcraft-geom.workspace = true\nserde = { version = \"1\" }\n\n\
             [lints]\nworkspace = true\n",
        )
        .unwrap();
        assert_eq!(m.name.as_deref(), Some("beamcraft-model"));
        assert!(m.publish_false);
        assert_eq!(
            deps(&m),
            [
                ("beamcraft-geom", "dependencies"),
                ("serde", "dependencies")
            ]
        );
    }

    #[test]
    fn reads_every_dependency_table_form() {
        let m = parse_manifest(
            "[dev-dependencies]\na = \"1\"\n[build-dependencies]\nb = \"1\"\n\
             [target.'cfg(unix)'.dependencies]\nc = \"1\"\n\
             [target.x86_64-pc-windows-msvc.dev-dependencies]\nd = \"1\"\n\
             [dependencies.e]\nversion = \"1\"\n\
             [dependencies.f]\npackage = \"real-f\"\n",
        )
        .unwrap();
        let names: Vec<&str> = m.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c", "d", "e", "real-f"]);
        assert_eq!(
            m.dependencies.get(2).map(|d| d.section.as_str()),
            Some("target.'cfg(unix)'.dependencies")
        );
    }

    #[test]
    fn renamed_dependency_uses_the_real_package_name() {
        let m = parse_manifest(
            "[dependencies]\nalias = { package = \"beamcraft-cli\", path = \"x\" }\n",
        )
        .unwrap();
        assert_eq!(deps(&m), [("beamcraft-cli", "dependencies")]);
    }

    #[test]
    fn workspace_tables_are_not_dependencies() {
        let m = parse_manifest(
            "[workspace]\nresolver = \"3\"\nmembers = [\n  \"crates/*\", # c\n  \"tools/fixtures\",\n]\n\
             [workspace.dependencies]\nbeamcraft-geom = { path = \"crates/geom\" }\n\
             [workspace.package]\nname = \"not-a-package\"\n",
        )
        .unwrap();
        assert_eq!(m.members, ["crates/*", "tools/fixtures"]);
        assert!(m.dependencies.is_empty());
        assert_eq!(m.name, None);
    }

    #[test]
    fn comments_and_hashes_inside_strings() {
        assert_eq!(strip_comment("a = \"x#y\" # z"), "a = \"x#y\" ");
        assert_eq!(strip_comment("# only"), "");
    }

    #[test]
    fn repeated_keys_of_one_dependency_count_once() {
        let m = parse_manifest("[dependencies]\nx.workspace = true\nx.optional = true\n").unwrap();
        assert_eq!(m.dependencies.len(), 1);
    }

    #[test]
    fn dotted_package_is_resolved_before_the_edge_check() {
        let m = parse_manifest(
            "[dependencies]\nbeamcraft-geom.package = \"tokio\"\nbeamcraft-geom.version = \"1\"\n\
             \"quoted\".package = \"serde\"\nplain.version = \"2\"\n",
        )
        .unwrap();
        assert_eq!(
            deps(&m),
            [
                ("tokio", "dependencies"),
                ("serde", "dependencies"),
                ("plain", "dependencies")
            ]
        );
    }

    #[test]
    fn dotted_package_after_other_fields_and_in_a_subtable() {
        let m = parse_manifest(
            "[dependencies]\nx.version = \"1\"\nx.package = \"real-x\"\n\
             [dependencies.y]\nversion = \"1\"\npackage = \"real-y\"\n",
        )
        .unwrap();
        assert_eq!(
            deps(&m),
            [("real-x", "dependencies"), ("real-y", "dependencies")]
        );
    }

    #[test]
    fn inherited_dependency_resolves_against_the_root_alias() {
        let root = parse_manifest(
            "[workspace.dependencies]\nbeamcraft-geom = { package = \"tokio\", version = \"1\" }\n\
             harmless = { package = \"beamcraft-geom\", path = \"crates/geom\" }\n\
             [workspace.dependencies.dotted]\npackage = \"serde\"\n\
             [workspace.dependencies.flat]\nversion = \"1\"\n",
        )
        .unwrap();
        assert_eq!(
            root.workspace_dependencies,
            [
                ("beamcraft-geom".to_owned(), "tokio".to_owned()),
                ("harmless".to_owned(), "beamcraft-geom".to_owned()),
                ("dotted".to_owned(), "serde".to_owned()),
                ("flat".to_owned(), "flat".to_owned())
            ]
        );
        assert!(root.dependencies.is_empty());
        let member = parse_manifest_in(
            "[dependencies]\nbeamcraft-geom.workspace = true\nharmless = { workspace = true }\n\
             dotted.workspace = true\nflat.workspace = true\nunknown.workspace = true\n",
            &root.workspace_dependencies,
        )
        .unwrap();
        assert_eq!(
            deps(&member),
            [
                ("tokio", "dependencies"),
                ("beamcraft-geom", "dependencies"),
                ("serde", "dependencies"),
                ("flat", "dependencies"),
                ("unknown", "dependencies")
            ]
        );
        let sub = parse_manifest_in(
            "[dependencies.beamcraft-geom]\nworkspace = true\n",
            &root.workspace_dependencies,
        )
        .unwrap();
        assert_eq!(deps(&sub), [("tokio", "dependencies")]);
    }

    #[test]
    fn reads_target_paths_and_the_build_key() {
        let m = parse_manifest(
            "[package]
name = \"x\"
build = \"gen.rs\"
             [lib]
path = \"code/lib.rs\" # moved
             [[bin]]
name = \"b\"
path = 'src/main.rs'
             [[test]]
path = \"tests/all.rs\"
             [dependencies]
path = \"not-a-target\"
",
        )
        .unwrap();
        assert_eq!(m.build.as_deref(), Some("gen.rs"));
        assert_eq!(
            m.targets,
            [
                ("lib".to_owned(), "code/lib.rs".to_owned()),
                ("bin".to_owned(), "src/main.rs".to_owned()),
                ("test".to_owned(), "tests/all.rs".to_owned()),
            ]
        );
        let unquoted = parse_manifest(
            "[lib]
path = concat
[package]
build = true
",
        )
        .unwrap();
        assert_eq!(unquoted.targets, [("lib".to_owned(), "concat".to_owned())]);
        assert_eq!(unquoted.build.as_deref(), Some("true"));
    }

    #[test]
    fn rejects_oversized_input() {
        let big = "#".repeat(MAX_MANIFEST_BYTES + 1);
        assert_eq!(parse_manifest(&big), Err(ManifestError::TooLarge));
        let many = "\n".repeat(MAX_LINES + 1);
        assert_eq!(parse_manifest(&many), Err(ManifestError::TooManyLines));
        assert!(parse_manifest(&"\n".repeat(MAX_LINES)).is_ok());
    }

    #[test]
    fn garbage_does_not_panic() {
        for text in [
            "[",
            "[[",
            "]",
            "=",
            "a = [",
            "[dependencies",
            "[workspace]\nmembers = [",
            "\"",
            "'''",
        ] {
            let _ = parse_manifest(text);
        }
    }
}
