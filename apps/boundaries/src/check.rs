// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The boundary rules, applied to an in-memory workspace and module table.
//!
//! Nothing here touches the disk, so every failing path is tested with synthetic input and no
//! broken manifest is ever committed.

use std::fmt;

use crate::table::{Kind, ModuleTable, Status};
use crate::workspace::Workspace;

/// APIs that a domain crate must not use outside test code (document section 1, rule 2).
pub const FORBIDDEN_APIS: [&str; 6] = [
    "std::fs",
    "std::process",
    "std::net",
    "std::thread",
    "std::env",
    "std::os",
];

/// One broken rule. The text of [`fmt::Display`] names the rule and the offending item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    /// Two table rows share a package name.
    DuplicateName(String),
    /// Two table rows share a path.
    DuplicatePath(String),
    /// A row allows a package that has no row.
    UnknownAllowed {
        /// Row package.
        package: String,
        /// The unknown dependency.
        dependency: String,
    },
    /// A row allows a dependency that is not in a strictly lower layer.
    NotLowerLayer {
        /// Row package.
        package: String,
        /// The offending dependency.
        dependency: String,
    },
    /// A workspace member has no row.
    MemberNotInTable {
        /// Member package.
        package: String,
        /// Member path.
        path: String,
    },
    /// A member's path differs from its row.
    PathMismatch {
        /// Member package.
        package: String,
        /// Path in the table.
        table: String,
        /// Path in the workspace.
        actual: String,
    },
    /// A member's row says `planned`.
    MemberNotNow(String),
    /// A `now` row has no workspace member.
    RowNotMember(String),
    /// An internal dependency that the row does not list.
    EdgeNotAllowed {
        /// Depending package.
        from: String,
        /// Dependency.
        to: String,
        /// Manifest table that declares it.
        section: String,
    },
    /// A WASM-required domain crate depends on a native library, host, app or tool.
    DomainDependsOnNative {
        /// Domain package.
        from: String,
        /// The non-domain dependency.
        to: String,
    },
    /// Some package depends on an app or a tool.
    DependsOnAppOrTool {
        /// Depending package.
        from: String,
        /// The app or tool.
        to: String,
    },
    /// A package other than an app or a tool depends on the application layer.
    DependsOnService {
        /// Depending package.
        from: String,
        /// The service crate.
        to: String,
    },
    /// A package other than an app, a service or a tool depends on a discipline.
    DependsOnDiscipline {
        /// Depending package.
        from: String,
        /// The discipline.
        to: String,
    },
    /// A domain crate uses a third-party crate that is not on the allowlist.
    ForbiddenThirdParty {
        /// Domain package.
        package: String,
        /// The third-party dependency.
        dependency: String,
        /// Manifest table that declares it.
        section: String,
    },
    /// A tool is publishable.
    ToolPublishable(String),
    /// Non-test source of a domain crate mentions a forbidden API.
    ForbiddenApi {
        /// Domain package.
        package: String,
        /// Source path.
        file: String,
        /// The mentioned API path.
        api: &'static str,
    },
    /// A target path leaves the standard layout, so its sources would not be scanned.
    TargetOutsideLayout {
        /// Package.
        package: String,
        /// The target table, for example `lib`.
        target: String,
        /// The declared path.
        path: String,
    },
    /// A domain crate has a build script.
    DomainBuildScript(String),
    /// Non-test source of a domain crate loads a module from a `#[path]`.
    DomainPathAttribute {
        /// Domain package.
        package: String,
        /// Source path.
        file: String,
    },
    /// The CI wasm check names crates by hand.
    CiHardCodedList(String),
    /// A CI job that checks wasm32 never asks the tool for the portable list.
    CiMissingPortableCall,
    /// No CI job mentions the wasm32 target.
    CiNoWasmJob,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateName(p) => write!(f, "table: package `{p}` has more than one row"),
            Self::DuplicatePath(p) => write!(f, "table: path `{p}` has more than one row"),
            Self::UnknownAllowed {
                package,
                dependency,
            } => {
                write!(
                    f,
                    "table: `{package}` allows `{dependency}`, which has no row"
                )
            }
            Self::NotLowerLayer {
                package,
                dependency,
            } => write!(
                f,
                "table: `{package}` allows `{dependency}`, which is not in a strictly lower layer"
            ),
            Self::MemberNotInTable { package, path } => {
                write!(
                    f,
                    "workspace member `{package}` ({path}) is missing from the module table"
                )
            }
            Self::PathMismatch {
                package,
                table,
                actual,
            } => write!(
                f,
                "`{package}` is at `{actual}` but the module table says `{table}`"
            ),
            Self::MemberNotNow(p) => {
                write!(
                    f,
                    "`{p}` is a workspace member but its table status is `planned`"
                )
            }
            Self::RowNotMember(p) => {
                write!(
                    f,
                    "table row `{p}` is `now` but it is not a workspace member"
                )
            }
            Self::EdgeNotAllowed { from, to, section } => write!(
                f,
                "`{from}` depends on `{to}` in [{section}], which its table row does not allow"
            ),
            Self::DomainDependsOnNative { from, to } => {
                write!(
                    f,
                    "domain crate `{from}` depends on non-domain crate `{to}`"
                )
            }
            Self::DependsOnAppOrTool { from, to } => {
                write!(f, "`{from}` depends on app or tool `{to}`")
            }
            Self::DependsOnService { from, to } => write!(
                f,
                "`{from}` depends on the application layer `{to}`; only an app or a tool may"
            ),
            Self::DependsOnDiscipline { from, to } => {
                write!(
                    f,
                    "`{from}` depends on discipline `{to}`; only an app or a tool may"
                )
            }
            Self::ForbiddenThirdParty {
                package,
                dependency,
                section,
            } => write!(
                f,
                "domain crate `{package}` depends on `{dependency}` in [{section}], which is not on the third-party allowlist"
            ),
            Self::ToolPublishable(p) => write!(f, "tool `{p}` must set publish = false"),
            Self::ForbiddenApi { package, file, api } => {
                write!(
                    f,
                    "domain crate `{package}` uses `{api}` in `{file}` outside test code"
                )
            }
            Self::TargetOutsideLayout {
                package,
                target,
                path,
            } => write!(
                f,
                "`{package}` declares its {target} at `{path}`, outside the standard layout \
                 (`src/` for lib and bin, `tests/`, `benches/`, `examples/`); its sources would \
                 not be checked"
            ),
            Self::DomainBuildScript(p) => write!(
                f,
                "domain crate `{p}` has a build script; it would run native code and could \
                 generate unchecked sources"
            ),
            Self::DomainPathAttribute { package, file } => write!(
                f,
                "domain crate `{package}` loads a module with `#[path]` in `{file}` outside test \
                 code; that module would not be checked"
            ),
            Self::CiHardCodedList(line) => write!(
                f,
                "CI wasm job selects a package by hand (`{}`); derive the list with `beamcraft-boundaries portable`",
                line.trim()
            ),
            Self::CiMissingPortableCall => f.write_str(
                "the CI wasm job never runs `beamcraft-boundaries -- portable` to derive its crate list",
            ),
            Self::CiNoWasmJob => f.write_str("no CI job checks the wasm32-unknown-unknown target"),
        }
    }
}

/// Checks the table itself: unique names and paths, known dependencies, strictly lower layers and
/// the kind rules.
pub fn check_table(table: &ModuleTable) -> Vec<Violation> {
    let mut out = Vec::new();
    for (i, row) in table.rows.iter().enumerate() {
        let later = || table.rows.iter().skip(i.saturating_add(1));
        if later().any(|r| r.package == row.package) {
            out.push(Violation::DuplicateName(row.package.clone()));
        }
        if later().any(|r| r.path == row.path) {
            out.push(Violation::DuplicatePath(row.path.clone()));
        }
        for dep in &row.allowed {
            let Some(target) = table.row(dep) else {
                out.push(Violation::UnknownAllowed {
                    package: row.package.clone(),
                    dependency: dep.clone(),
                });
                continue;
            };
            if target.layer >= row.layer {
                out.push(Violation::NotLowerLayer {
                    package: row.package.clone(),
                    dependency: dep.clone(),
                });
            }
            out.extend(kind_rules(
                &row.package,
                row.kind,
                &target.package,
                target.kind,
            ));
        }
    }
    out
}

fn kind_rules(from: &str, from_kind: Kind, to: &str, to_kind: Kind) -> Vec<Violation> {
    let mut out = Vec::new();
    if from_kind.requires_wasm() && !to_kind.requires_wasm() {
        out.push(Violation::DomainDependsOnNative {
            from: from.to_owned(),
            to: to.to_owned(),
        });
    }
    if matches!(to_kind, Kind::App | Kind::Tool) {
        out.push(Violation::DependsOnAppOrTool {
            from: from.to_owned(),
            to: to.to_owned(),
        });
    }
    if to_kind == Kind::Service && !matches!(from_kind, Kind::App | Kind::Tool) {
        out.push(Violation::DependsOnService {
            from: from.to_owned(),
            to: to.to_owned(),
        });
    }
    if to_kind == Kind::Discipline && !matches!(from_kind, Kind::Service | Kind::App | Kind::Tool) {
        out.push(Violation::DependsOnDiscipline {
            from: from.to_owned(),
            to: to.to_owned(),
        });
    }
    out
}

/// Source with comments and string/char literals blanked (newlines kept), so that braces and
/// paths inside them cannot confuse the item scan.
fn blank_literals(source: &str) -> Vec<char> {
    let chars: Vec<char> = source.chars().collect();
    let at = |i: usize| chars.get(i).copied().unwrap_or('\0');
    let blank = |c: char| if c == '\n' { '\n' } else { ' ' };
    let mut out = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let c = at(i);
        if c == '/' && at(i + 1) == '/' {
            while i < chars.len() && at(i) != '\n' {
                out.push(' ');
                i += 1;
            }
        } else if c == '/' && at(i + 1) == '*' {
            let mut depth = 0usize;
            while i < chars.len() {
                if at(i) == '/' && at(i + 1) == '*' {
                    depth += 1;
                    out.extend([' ', ' ']);
                    i += 2;
                } else if at(i) == '*' && at(i + 1) == '/' {
                    depth = depth.saturating_sub(1);
                    out.extend([' ', ' ']);
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    out.push(blank(at(i)));
                    i += 1;
                }
            }
        } else if c == '"' || (c == 'r' && raw_start(&chars, i)) {
            // Ordinary string or raw string with any number of hashes.
            let raw = c == 'r';
            let mut hashes = 0usize;
            let mut j = i;
            if raw {
                j += 1;
                while at(j) == '#' {
                    hashes += 1;
                    j += 1;
                }
            }
            // `j` is at the opening quote.
            j += 1;
            while j < chars.len() {
                let d = at(j);
                if !raw && d == '\\' {
                    j += 2;
                    continue;
                }
                if d == '"' && (0..hashes).all(|k| at(j + 1 + k) == '#') {
                    j += 1 + hashes;
                    break;
                }
                j += 1;
            }
            let end = j.min(chars.len());
            while i < end {
                out.push(blank(at(i)));
                i += 1;
            }
        } else if c == '\'' {
            // A char literal is a quote, one character or an escape, and a quote; anything
            // else is a lifetime.
            let len = if at(i + 1) == '\\' {
                (i + 3..chars.len().min(i + 12))
                    .find(|j| at(*j) == '\'')
                    .map(|j| j - i + 1)
            } else if at(i + 2) == '\'' && at(i + 1) != '\'' {
                Some(3)
            } else {
                None
            };
            match len {
                Some(n) => {
                    out.extend(std::iter::repeat_n(' ', n));
                    i += n;
                }
                None => {
                    out.push(c);
                    i += 1;
                }
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// True if the `r` at `i` starts a raw string (`r"` or `r#"`), not an identifier ending in `r`.
fn raw_start(chars: &[char], i: usize) -> bool {
    let at = |k: usize| chars.get(k).copied().unwrap_or('\0');
    if i > 0 && (at(i - 1).is_alphanumeric() || at(i - 1) == '_') {
        return false;
    }
    let mut j = i + 1;
    while at(j) == '#' {
        j += 1;
    }
    at(j) == '"'
}

/// The attribute that starts at `from` (a `#`), without whitespace, and the index after it.
fn attribute(code: &[char], from: usize) -> Option<(String, usize)> {
    let mut text = String::new();
    let mut depth = 0usize;
    let mut j = from;
    while let Some(&c) = code.get(j) {
        if c == '[' {
            depth += 1;
        } else if c == ']' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                text.push(c);
                return Some((text, j + 1));
            }
        }
        if !c.is_whitespace() {
            text.push(c);
        }
        j += 1;
    }
    None
}

/// Index just after the `}` that closes the `{` at `start` (the end of `code` if unclosed).
fn block_end(code: &[char], start: usize) -> usize {
    let mut depth = 0usize;
    let mut j = start;
    while let Some(&c) = code.get(j) {
        j += 1;
        if c == '{' {
            depth += 1;
        } else if c == '}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                break;
            }
        }
    }
    j
}

/// Replaces every character in `from..to` except newlines by a space.
fn blank(code: &mut [char], from: usize, to: usize) {
    for c in code.iter_mut().take(to).skip(from) {
        if *c != '\n' {
            *c = ' ';
        }
    }
}

/// Blanks every item annotated `#[cfg(test)]` (a `mod`, `fn`, `use`, ... with its body), wherever
/// it appears. An inner `#![cfg(test)]` exempts only the scope that contains it: the whole file
/// when it appears at file level before any item, otherwise just the enclosing `{ ... }` block
/// (an inline `mod tests`). A misplaced file-level inner attribute after an item is ignored, so
/// the code stays scanned. Production code before, between and after test items stays visible,
/// so ordering cannot hide a forbidden API.
fn without_test_items(code: &mut [char]) {
    let mut i = 0;
    let mut open: Vec<usize> = Vec::new();
    let mut item_seen = false;
    while let Some(&c) = code.get(i) {
        match c {
            '{' => {
                open.push(i);
                item_seen = true;
                i += 1;
            }
            '}' => {
                open.pop();
                i += 1;
            }
            '#' => {
                let Some((text, after)) = attribute(code, i) else {
                    break;
                };
                if text == "#![cfg(test)]" {
                    if let Some(start) = open.pop() {
                        let end = block_end(code, start);
                        blank(code, start, end);
                        i = end;
                    } else if item_seen {
                        i = after;
                    } else {
                        blank(code, 0, code.len());
                        return;
                    }
                    continue;
                }
                if text != "#[cfg(test)]" {
                    i = after;
                    continue;
                }
                item_seen = true;
                // Consume the annotated item: up to a `;` or the end of its first brace block.
                let mut j = after;
                let (mut brace, mut other) = (0usize, 0usize);
                let mut opened = false;
                while let Some(&c) = code.get(j) {
                    j += 1;
                    match c {
                        '{' => {
                            brace += 1;
                            opened = true;
                        }
                        '}' => {
                            brace = brace.saturating_sub(1);
                            if opened && brace == 0 && other == 0 {
                                break;
                            }
                        }
                        '(' | '[' => other += 1,
                        ')' | ']' => other = other.saturating_sub(1),
                        ';' if brace == 0 && other == 0 => break,
                        _ => {}
                    }
                }
                let end = j.min(code.len());
                blank(code, i, end);
                i = end;
            }
            c => {
                item_seen |= !c.is_whitespace();
                i += 1;
            }
        }
    }
}

/// Identifiers that open each top-level entry of every `std::{ ... }` group in `code`.
fn std_group_heads(code: &str) -> Vec<String> {
    let mut heads = Vec::new();
    for (at, _) in code.match_indices("std::{") {
        let rest = code.get(at + "std::{".len()..).unwrap_or("");
        let (mut depth, mut head, mut fresh) = (1usize, String::new(), true);
        for c in rest.chars() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        break;
                    }
                }
                ',' if depth == 1 => {
                    heads.push(std::mem::take(&mut head));
                    fresh = true;
                }
                c if depth == 1 && fresh && (c.is_alphanumeric() || c == '_') => head.push(c),
                // Whitespace after a non-empty head ends it, so `fs as f` is `fs`, not `fsasf`.
                c if depth == 1 && c.is_whitespace() && !head.is_empty() => fresh = false,
                c if depth == 1 && !c.is_whitespace() => fresh = false,
                _ => {}
            }
        }
        heads.push(head);
    }
    heads
}

/// The forbidden APIs that non-test source mentions, in order of first appearance.
///
/// Comments and string literals are ignored. Every item annotated `#[cfg(test)]` is excluded
/// wherever it is declared, and only that item: production code after a test module is still
/// scanned. A `use std::{fs, io}` group is read as `std::fs` and `std::io`.
pub fn forbidden_apis(source: &str) -> Vec<&'static str> {
    let mut code = blank_literals(source);
    without_test_items(&mut code);
    let code: String = code.into_iter().collect();
    let mut found = Vec::new();
    for line in code.lines() {
        for api in FORBIDDEN_APIS {
            let mentioned = line.match_indices(api).any(|(at, _)| {
                let after = line.get(at.saturating_add(api.len())..).unwrap_or("");
                !after.starts_with(|c: char| c.is_alphanumeric() || c == '_')
            });
            if mentioned && !found.contains(&api) {
                found.push(api);
            }
        }
    }
    for head in std_group_heads(&code) {
        for api in FORBIDDEN_APIS {
            if api.strip_prefix("std::") == Some(head.as_str()) && !found.contains(&api) {
                found.push(api);
            }
        }
    }
    found
}

/// The directory a target table's path must stay inside.
fn layout_dir(target: &str) -> &'static str {
    match target {
        "test" => "tests/",
        "bench" => "benches/",
        "example" => "examples/",
        _ => "src/",
    }
}

/// Whether `path` is a plain relative path inside the standard directory of `target`: `/`
/// separators, no empty, `.` or `..` component, ending in `.rs`.
pub fn inside_layout(target: &str, path: &str) -> bool {
    path.strip_prefix(layout_dir(target)).is_some_and(|rest| {
        path.ends_with(".rs")
            && !path.contains('\\')
            && rest
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != "..")
    })
}

/// Whether non-test source loads a module from a `#[path = ...]` attribute.
pub fn has_path_attribute(source: &str) -> bool {
    let mut code = blank_literals(source);
    without_test_items(&mut code);
    let code: String = code.into_iter().filter(|c| !c.is_whitespace()).collect();
    code.contains("#[path=") || code.contains("#![path=")
}

/// Checks a workspace against the module table.
pub fn check_workspace(workspace: &Workspace, table: &ModuleTable) -> Vec<Violation> {
    let mut out = check_table(table);
    for package in &workspace.packages {
        let Some(row) = table.row(&package.name) else {
            out.push(Violation::MemberNotInTable {
                package: package.name.clone(),
                path: package.path.clone(),
            });
            continue;
        };
        if row.path != package.path {
            out.push(Violation::PathMismatch {
                package: package.name.clone(),
                table: row.path.clone(),
                actual: package.path.clone(),
            });
        }
        if row.status != Status::Now {
            out.push(Violation::MemberNotNow(package.name.clone()));
        }
        if row.kind == Kind::Tool && !package.publish_false {
            out.push(Violation::ToolPublishable(package.name.clone()));
        }
        for (target, path) in &package.targets {
            if !inside_layout(target, path) {
                out.push(Violation::TargetOutsideLayout {
                    package: package.name.clone(),
                    target: target.clone(),
                    path: path.clone(),
                });
            }
        }
        if row.kind.requires_wasm() && package.build_script {
            out.push(Violation::DomainBuildScript(package.name.clone()));
        }
        for dep in &package.dependencies {
            match table.row(&dep.name) {
                Some(target) if target.package != package.name => {
                    if !row.allowed.contains(&dep.name) {
                        out.push(Violation::EdgeNotAllowed {
                            from: package.name.clone(),
                            to: dep.name.clone(),
                            section: dep.section.clone(),
                        });
                    }
                    out.extend(kind_rules(
                        &package.name,
                        row.kind,
                        &target.package,
                        target.kind,
                    ));
                }
                Some(_) => out.push(Violation::EdgeNotAllowed {
                    from: package.name.clone(),
                    to: dep.name.clone(),
                    section: dep.section.clone(),
                }),
                None if row.kind.requires_wasm() && !table.third_party.contains(&dep.name) => {
                    out.push(Violation::ForbiddenThirdParty {
                        package: package.name.clone(),
                        dependency: dep.name.clone(),
                        section: dep.section.clone(),
                    });
                }
                None => {}
            }
        }
        if row.kind.requires_wasm() {
            for source in &package.sources {
                for api in forbidden_apis(&source.text) {
                    out.push(Violation::ForbiddenApi {
                        package: package.name.clone(),
                        file: source.path.clone(),
                        api,
                    });
                }
                if has_path_attribute(&source.text) {
                    out.push(Violation::DomainPathAttribute {
                        package: package.name.clone(),
                        file: source.path.clone(),
                    });
                }
            }
        }
    }
    for row in &table.rows {
        if row.status == Status::Now && !workspace.packages.iter().any(|p| p.name == row.package) {
            out.push(Violation::RowNotMember(row.package.clone()));
        }
    }
    out
}

/// Leading spaces of a line.
fn indent(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

/// Logical lines of each job under `jobs:` whose text mentions the wasm32 target.
///
/// Comment lines are dropped and lines ending in `\` are joined with the next line, so a
/// continuation cannot hide an argument from the scan.
fn wasm_job_lines(ci: &str) -> Vec<Vec<String>> {
    let mut jobs: Vec<Vec<&str>> = Vec::new();
    let mut job_indent: Option<usize> = None;
    let mut in_jobs = false;
    for line in ci.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if indent(line) == 0 {
            in_jobs = trimmed == "jobs:";
            job_indent = None;
            continue;
        }
        if !in_jobs {
            continue;
        }
        let first = *job_indent.get_or_insert(indent(line));
        if indent(line) == first {
            jobs.push(Vec::new());
        }
        if let Some(job) = jobs.last_mut() {
            job.push(line);
        }
    }
    jobs.into_iter()
        .filter(|job| job.iter().any(|l| l.contains("wasm32-unknown-unknown")))
        .map(|job| {
            let mut logical = Vec::new();
            let mut pending = String::new();
            for line in job {
                let trimmed = line.trim();
                match trimmed.strip_suffix('\\') {
                    Some(head) => {
                        pending.push_str(head);
                        pending.push(' ');
                    }
                    None => {
                        pending.push_str(trimmed);
                        logical.push(std::mem::take(&mut pending));
                    }
                }
            }
            if !pending.is_empty() {
                logical.push(pending);
            }
            logical
        })
        .collect()
}

/// True if a logical line passes a package selection (`-p`, `-pNAME`, `--package`) to cargo that
/// is not the tool's own `cargo run -p beamcraft-boundaries`.
fn hand_written_package_arg(line: &str) -> bool {
    let spaced: String = line
        .chars()
        .map(|c| if "\"'()=;`".contains(c) { ' ' } else { c })
        .collect();
    let tokens: Vec<&str> = spaced.split_whitespace().collect();
    let runs_tool = line.contains("cargo run");
    tokens.iter().enumerate().any(|(i, token)| {
        let argument = if *token == "-p" || *token == "--package" {
            tokens.get(i + 1).copied().unwrap_or("")
        } else if let Some(rest) = token
            .strip_prefix("-p")
            .filter(|_| !token.starts_with("--"))
        {
            rest
        } else {
            return false;
        };
        !(runs_tool && argument == "beamcraft-boundaries")
    })
}

/// Checks that the CI wasm job derives its crate list from the tool instead of repeating it.
///
/// Every job that mentions `wasm32-unknown-unknown` is read as a whole, with continuation lines
/// joined: it must call `beamcraft-boundaries -- portable`, and no line of it may select a
/// package by hand (`-p`, `-pNAME`, `--package`) except the tool's own `cargo run -p`.
pub fn check_ci(ci: &str) -> Vec<Violation> {
    let jobs = wasm_job_lines(ci);
    if jobs.is_empty() {
        return vec![Violation::CiNoWasmJob];
    }
    let mut out = Vec::new();
    for lines in jobs {
        for line in &lines {
            if hand_written_package_arg(line) {
                out.push(Violation::CiHardCodedList(line.clone()));
            }
        }
        if !lines
            .iter()
            .any(|l| l.contains("beamcraft-boundaries -- portable"))
        {
            out.push(Violation::CiMissingPortableCall);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Dependency;
    use crate::table::Row;
    use crate::workspace::{Package, Source};

    fn row(
        package: &str,
        path: &str,
        kind: Kind,
        layer: u8,
        allowed: &[&str],
        status: Status,
    ) -> Row {
        Row {
            package: package.into(),
            path: path.into(),
            kind,
            layer,
            allowed: allowed.iter().map(|s| (*s).into()).collect(),
            status,
            owner: "T".into(),
        }
    }

    fn table() -> ModuleTable {
        ModuleTable {
            rows: vec![
                row("geom", "crates/geom", Kind::Domain, 0, &[], Status::Now),
                row(
                    "model",
                    "crates/model",
                    Kind::Domain,
                    1,
                    &["geom"],
                    Status::Now,
                ),
                row(
                    "host",
                    "crates/host",
                    Kind::Host,
                    4,
                    &["model"],
                    Status::Planned,
                ),
                row(
                    "cli",
                    "apps/cli",
                    Kind::App,
                    5,
                    &["model", "host"],
                    Status::Now,
                ),
                row("fix", "tools/fix", Kind::Tool, 9, &[], Status::Now),
            ],
            third_party: vec!["allowed-lib".into()],
        }
    }

    fn pkg(name: &str, path: &str, deps: &[&str]) -> Package {
        Package {
            name: name.into(),
            path: path.into(),
            publish_false: name == "fix",
            dependencies: deps
                .iter()
                .map(|d| Dependency {
                    name: (*d).into(),
                    section: "dependencies".into(),
                })
                .collect(),
            sources: Vec::new(),
            targets: Vec::new(),
            build_script: false,
        }
    }

    fn good() -> Workspace {
        Workspace {
            packages: vec![
                pkg("geom", "crates/geom", &[]),
                pkg("model", "crates/model", &["geom"]),
                pkg("cli", "apps/cli", &["model"]),
                pkg("fix", "tools/fix", &[]),
            ],
        }
    }

    fn with(mut ws: Workspace, edit: impl FnOnce(&mut Workspace)) -> Vec<Violation> {
        edit(&mut ws);
        check_workspace(&ws, &table())
    }

    fn pkg_mut<'a>(ws: &'a mut Workspace, name: &str) -> &'a mut Package {
        ws.packages.iter_mut().find(|p| p.name == name).unwrap()
    }

    #[test]
    fn a_consistent_workspace_passes() {
        assert_eq!(check_workspace(&good(), &table()), []);
    }

    #[test]
    fn rejects_a_back_edge() {
        let v = with(good(), |ws| {
            pkg_mut(ws, "geom").dependencies.push(Dependency {
                name: "model".into(),
                section: "dependencies".into(),
            })
        });
        assert!(v.contains(&Violation::EdgeNotAllowed {
            from: "geom".into(),
            to: "model".into(),
            section: "dependencies".into()
        }));
    }

    #[test]
    fn rejects_an_edge_in_a_dev_dependency_table_too() {
        let v = with(good(), |ws| {
            pkg_mut(ws, "model").dependencies.push(Dependency {
                name: "cli".into(),
                section: "dev-dependencies".into(),
            })
        });
        assert!(v.iter().any(|x| matches!(x, Violation::EdgeNotAllowed { section, .. } if section == "dev-dependencies")));
        assert!(v.contains(&Violation::DependsOnAppOrTool {
            from: "model".into(),
            to: "cli".into()
        }));
        assert!(v.contains(&Violation::DomainDependsOnNative {
            from: "model".into(),
            to: "cli".into()
        }));
    }

    #[test]
    fn rejects_a_domain_crate_depending_on_the_host() {
        let v = with(good(), |ws| {
            pkg_mut(ws, "model").dependencies.push(Dependency {
                name: "host".into(),
                section: "dependencies".into(),
            })
        });
        assert!(v.contains(&Violation::DomainDependsOnNative {
            from: "model".into(),
            to: "host".into()
        }));
    }

    #[test]
    fn wasm_required_crates_reject_native_only_dependencies_in_every_manifest_section() {
        let mut table = table();
        table.rows.push(row(
            "native",
            "crates/native",
            Kind::Native,
            0,
            &[],
            Status::Now,
        ));
        table
            .rows
            .iter_mut()
            .find(|r| r.package == "model")
            .unwrap()
            .allowed
            .push("native".into());
        let expected = Violation::DomainDependsOnNative {
            from: "model".into(),
            to: "native".into(),
        };
        assert_eq!(
            check_table(&table).as_slice(),
            std::slice::from_ref(&expected)
        );
        for section in [
            "dependencies",
            "dev-dependencies",
            "build-dependencies",
            "target.'cfg(windows)'.dependencies",
        ] {
            let mut ws = good();
            ws.packages.push(pkg("native", "crates/native", &[]));
            pkg_mut(&mut ws, "model").dependencies.push(Dependency {
                name: "native".into(),
                section: section.into(),
            });
            // One violation comes from the table itself, one from the manifest edge.
            let matching = check_workspace(&ws, &table)
                .iter()
                .filter(|v| **v == expected)
                .count();
            assert_eq!(matching, 2, "{section}");
        }
    }

    #[test]
    fn only_an_app_or_a_tool_may_depend_on_a_discipline() {
        let breach = |from: &str, kind: Kind, to: &str| {
            kind_rules(from, kind, to, Kind::Discipline).contains(&Violation::DependsOnDiscipline {
                from: from.into(),
                to: to.into(),
            })
        };
        // The platform never depends on a discipline, and a discipline never on another.
        for kind in [Kind::Domain, Kind::Native, Kind::Host, Kind::Discipline] {
            assert!(breach("p", kind, "steel"), "{kind:?}");
        }
        // The application layer, an app or a tool may; a discipline may depend on platform
        // crates.
        for kind in [Kind::Service, Kind::App, Kind::Tool] {
            assert!(!breach("p", kind, "steel"), "{kind:?}");
        }
        for to in [Kind::Domain, Kind::Native, Kind::Host] {
            assert_eq!(kind_rules("steel", Kind::Discipline, "p", to), [], "{to:?}");
        }
        assert!(!Kind::Discipline.requires_wasm());
    }

    #[test]
    fn only_an_app_or_a_tool_may_depend_on_the_application_layer() {
        let breach = |kind: Kind| {
            kind_rules("p", kind, "commands", Kind::Service).contains(
                &Violation::DependsOnService {
                    from: "p".into(),
                    to: "commands".into(),
                },
            )
        };
        for kind in [
            Kind::Domain,
            Kind::Native,
            Kind::Host,
            Kind::Discipline,
            Kind::Service,
        ] {
            assert!(breach(kind), "{kind:?}");
        }
        for kind in [Kind::App, Kind::Tool] {
            assert!(!breach(kind), "{kind:?}");
        }
        // The application layer itself may use the platform and the disciplines.
        for to in [Kind::Native, Kind::Host, Kind::Discipline] {
            assert_eq!(kind_rules("commands", Kind::Service, "p", to), [], "{to:?}");
        }
        assert!(!Kind::Service.requires_wasm());
    }

    #[test]
    fn desktop_crates_need_not_have_wasm_clean_source_or_dependencies() {
        for kind in [
            Kind::Native,
            Kind::Host,
            Kind::Discipline,
            Kind::App,
            Kind::Tool,
        ] {
            let mut table = table();
            table.rows.push(row(
                "desktop",
                "crates/desktop",
                kind,
                3,
                &["model"],
                Status::Now,
            ));
            let mut ws = good();
            let mut desktop = pkg("desktop", "crates/desktop", &["model", "native-only-lib"]);
            desktop.publish_false = true;
            desktop.sources.push(Source {
                path: "src/lib.rs".into(),
                text: "use std::{fs, process, net, thread, env, os};".into(),
            });
            ws.packages.push(desktop);
            assert_eq!(check_workspace(&ws, &table), [], "{kind:?}");
            assert!(!table.portable().contains(&"desktop"));
        }
    }

    #[test]
    fn rejects_an_unlisted_forward_edge() {
        let v = with(good(), |ws| {
            pkg_mut(ws, "cli").dependencies.push(Dependency {
                name: "geom".into(),
                section: "dependencies".into(),
            })
        });
        assert!(v.contains(&Violation::EdgeNotAllowed {
            from: "cli".into(),
            to: "geom".into(),
            section: "dependencies".into()
        }));
    }

    #[test]
    fn rejects_a_self_dependency() {
        let v = with(good(), |ws| {
            pkg_mut(ws, "model").dependencies.push(Dependency {
                name: "model".into(),
                section: "dependencies".into(),
            })
        });
        assert!(v.iter().any(|x| matches!(x, Violation::EdgeNotAllowed { from, to, .. } if from == "model" && to == "model")));
    }

    #[test]
    fn domain_crates_may_use_only_allowlisted_third_party_crates() {
        let v = with(good(), |ws| {
            let m = pkg_mut(ws, "model");
            for name in ["tokio", "allowed-lib"] {
                m.dependencies.push(Dependency {
                    name: name.into(),
                    section: "dependencies".into(),
                });
            }
        });
        assert_eq!(
            v,
            [Violation::ForbiddenThirdParty {
                package: "model".into(),
                dependency: "tokio".into(),
                section: "dependencies".into()
            }]
        );
    }

    #[test]
    fn a_dotted_alias_is_checked_by_its_real_package() {
        use crate::manifest::{parse_manifest, parse_manifest_in};
        // `geom.package = "tokio"` hides an unlisted package behind an allowed internal name.
        let hidden =
            parse_manifest("[dependencies]\ngeom.package = \"tokio\"\ngeom.version = \"1\"\n")
                .unwrap();
        let v = with(good(), |ws| {
            pkg_mut(ws, "model").dependencies = hidden.dependencies;
        });
        assert_eq!(
            v,
            [Violation::ForbiddenThirdParty {
                package: "model".into(),
                dependency: "tokio".into(),
                section: "dependencies".into()
            }]
        );
        // The same through an inherited root alias.
        let root = parse_manifest(
            "[workspace.dependencies]\ngeom = { package = \"tokio\", version = \"1\" }\n\
             g = { package = \"geom\", path = \"crates/geom\" }\n",
        )
        .unwrap();
        let member = parse_manifest_in(
            "[dependencies]\ngeom.workspace = true\n",
            &root.workspace_dependencies,
        )
        .unwrap();
        let v = with(good(), |ws| {
            pkg_mut(ws, "model").dependencies = member.dependencies;
        });
        assert_eq!(
            v,
            [Violation::ForbiddenThirdParty {
                package: "model".into(),
                dependency: "tokio".into(),
                section: "dependencies".into()
            }]
        );
        // A harmless alias of an allowed internal package is not a third-party dependency.
        let harmless = parse_manifest_in(
            "[dependencies]\ng.workspace = true\n",
            &root.workspace_dependencies,
        )
        .unwrap();
        let v = with(good(), |ws| {
            pkg_mut(ws, "model").dependencies = harmless.dependencies;
        });
        assert_eq!(v, []);
    }

    #[test]
    fn apps_may_use_third_party_crates() {
        let v = with(good(), |ws| {
            pkg_mut(ws, "cli").dependencies.push(Dependency {
                name: "clap".into(),
                section: "dependencies".into(),
            })
        });
        assert_eq!(v, []);
    }

    #[test]
    fn rejects_a_member_missing_from_the_table() {
        let v = with(good(), |ws| {
            ws.packages.push(pkg("extra", "crates/extra", &[]))
        });
        assert_eq!(
            v,
            [Violation::MemberNotInTable {
                package: "extra".into(),
                path: "crates/extra".into()
            }]
        );
    }

    #[test]
    fn rejects_a_now_row_without_a_member() {
        let v = with(good(), |ws| ws.packages.retain(|p| p.name != "fix"));
        assert_eq!(v, [Violation::RowNotMember("fix".into())]);
    }

    #[test]
    fn rejects_a_planned_row_that_already_exists_and_a_moved_path() {
        let v = with(good(), |ws| {
            ws.packages.push(pkg("host", "crates/elsewhere", &[]))
        });
        assert!(v.contains(&Violation::MemberNotNow("host".into())));
        assert!(v.contains(&Violation::PathMismatch {
            package: "host".into(),
            table: "crates/host".into(),
            actual: "crates/elsewhere".into()
        }));
    }

    #[test]
    fn rejects_a_publishable_tool() {
        let v = with(good(), |ws| pkg_mut(ws, "fix").publish_false = false);
        assert_eq!(v, [Violation::ToolPublishable("fix".into())]);
    }

    #[test]
    fn rejects_forbidden_apis_in_domain_sources_only_outside_tests() {
        let v = with(good(), |ws| {
            pkg_mut(ws, "model").sources.push(Source {
                path: "crates/model/src/lib.rs".into(),
                text: "// std::fs in a comment is fine\nuse std::fs::File;\nfn f() { let _ = std::env::args(); }\n#[cfg(test)]\nmod t { use std::process::Command; }\n".into(),
            });
            pkg_mut(ws, "cli").sources.push(Source {
                path: "apps/cli/src/main.rs".into(),
                text: "use std::fs;\n".into(),
            });
        });
        let apis: Vec<&str> = v
            .iter()
            .filter_map(|x| match x {
                Violation::ForbiddenApi { api, .. } => Some(*api),
                _ => None,
            })
            .collect();
        assert_eq!(apis, ["std::fs", "std::env"]);
    }

    #[test]
    fn target_paths_must_stay_in_the_standard_layout() {
        for (target, path) in [
            ("lib", "src/lib.rs"),
            ("bin", "src/bin/tool.rs"),
            ("test", "tests/all.rs"),
            ("bench", "benches/speed.rs"),
            ("example", "examples/demo.rs"),
        ] {
            assert!(inside_layout(target, path), "{target} {path}");
        }
        for (target, path) in [
            ("lib", "code/lib.rs"),
            ("lib", "src/../code/lib.rs"),
            ("lib", "./src/lib.rs"),
            ("lib", "src//lib.rs"),
            ("lib", "src\\lib.rs"),
            ("lib", "/abs/src/lib.rs"),
            ("lib", "src/lib.txt"),
            ("lib", "concat"),
            ("bin", "tests/main.rs"),
            ("test", "src/t.rs"),
        ] {
            assert!(!inside_layout(target, path), "{target} {path}");
        }
        let v = with(good(), |ws| {
            let cli = pkg_mut(ws, "cli");
            cli.targets.push(("bin".into(), "src/main.rs".into()));
            cli.targets.push(("test".into(), "tests/cli.rs".into()));
        });
        assert_eq!(v, []);
    }

    /// The #208 reproduction: geom's sources moved to `code/` behind `[lib] path`, with
    /// `std::fs` in them. The scan of `src/` finds nothing, so the layout rule must fire.
    #[test]
    fn a_library_moved_out_of_src_is_a_violation() {
        let v = with(good(), |ws| {
            pkg_mut(ws, "geom")
                .targets
                .push(("lib".into(), "code/lib.rs".into()));
        });
        assert_eq!(
            v,
            [Violation::TargetOutsideLayout {
                package: "geom".into(),
                target: "lib".into(),
                path: "code/lib.rs".into(),
            }]
        );
        assert!(v[0].to_string().contains("outside the standard layout"));
    }

    #[test]
    fn a_domain_build_script_is_a_violation_and_an_app_one_is_not() {
        let v = with(good(), |ws| {
            pkg_mut(ws, "model").build_script = true;
            pkg_mut(ws, "cli").build_script = true;
        });
        assert_eq!(v, [Violation::DomainBuildScript("model".into())]);
    }

    #[test]
    fn a_domain_path_attribute_outside_tests_is_a_violation() {
        assert!(has_path_attribute("#[path = \"../code/x.rs\"]\nmod x;\n"));
        assert!(has_path_attribute("#[ path=\"x.rs\" ]\nmod x;\n"));
        assert!(!has_path_attribute(
            "// #[path = \"x.rs\"]\nconst S: &str = \"#[path = 1]\";\n"
        ));
        assert!(!has_path_attribute(
            "#[cfg(test)]\n#[path = \"tests.rs\"]\nmod tests;\n"
        ));
        let v = with(good(), |ws| {
            pkg_mut(ws, "geom").sources.push(Source {
                path: "crates/geom/src/lib.rs".into(),
                text: "#[path = \"../../elsewhere.rs\"]\nmod hidden;\n".into(),
            });
            pkg_mut(ws, "cli").sources.push(Source {
                path: "apps/cli/src/main.rs".into(),
                text: "#[path = \"other.rs\"]\nmod other;\n".into(),
            });
        });
        assert_eq!(
            v,
            [Violation::DomainPathAttribute {
                package: "geom".into(),
                file: "crates/geom/src/lib.rs".into(),
            }]
        );
    }

    #[test]
    fn table_rules_catch_bad_layers_unknown_names_and_duplicates() {
        let mut t = table();
        t.rows.push(row(
            "geom",
            "crates/other",
            Kind::Domain,
            0,
            &["ghost"],
            Status::Planned,
        ));
        t.rows.push(row(
            "loop",
            "crates/loop",
            Kind::Domain,
            1,
            &["model", "geom", "loop"],
            Status::Planned,
        ));
        t.rows.push(row(
            "up",
            "crates/up",
            Kind::Domain,
            1,
            &["cli", "fix"],
            Status::Planned,
        ));
        let v = check_table(&t);
        assert!(v.contains(&Violation::DuplicateName("geom".into())));
        assert!(v.contains(&Violation::UnknownAllowed {
            package: "geom".into(),
            dependency: "ghost".into()
        }));
        assert!(v.contains(&Violation::NotLowerLayer {
            package: "loop".into(),
            dependency: "model".into()
        }));
        assert!(v.contains(&Violation::NotLowerLayer {
            package: "loop".into(),
            dependency: "loop".into()
        }));
        assert!(v.contains(&Violation::NotLowerLayer {
            package: "up".into(),
            dependency: "cli".into()
        }));
        assert!(v.contains(&Violation::DomainDependsOnNative {
            from: "up".into(),
            to: "cli".into()
        }));
        assert!(v.contains(&Violation::DependsOnAppOrTool {
            from: "up".into(),
            to: "fix".into()
        }));
        let dup_path = {
            let mut t = table();
            t.rows.push(row(
                "z",
                "crates/geom",
                Kind::Domain,
                0,
                &[],
                Status::Planned,
            ));
            check_table(&t)
        };
        assert_eq!(dup_path, [Violation::DuplicatePath("crates/geom".into())]);
        assert_eq!(check_table(&table()), []);
    }

    #[test]
    fn api_names_must_match_whole_paths() {
        assert_eq!(
            forbidden_apis(
                "use std::fsx::Y;
use std::environment;
"
            ),
            Vec::<&str>::new()
        );
        assert_eq!(
            forbidden_apis(
                "use std::net::TcpStream;
"
            ),
            ["std::net"]
        );
        assert_eq!(
            forbidden_apis(
                "let t = std::thread::spawn(f);
"
            ),
            ["std::thread"]
        );
    }

    fn apis(source: &str) -> Vec<&'static str> {
        forbidden_apis(source)
    }

    #[test]
    fn a_forbidden_api_before_a_test_module_is_found() {
        assert_eq!(
            apis("use std::fs;\n#[cfg(test)]\nmod tests {\n    use std::process::Command;\n}\n"),
            ["std::fs"]
        );
    }

    #[test]
    fn a_forbidden_api_inside_a_test_item_is_allowed() {
        assert_eq!(
            apis(
                "#[cfg(test)]\nmod tests {\n    use std::fs;\n    fn f() { let _ = \"}\"; let _ = '}'; }\n    use std::net::X;\n}\n"
            ),
            Vec::<&str>::new()
        );
        assert_eq!(apis("#[cfg(test)]\nuse std::fs;\n"), Vec::<&str>::new());
        assert_eq!(
            apis(
                "#[cfg(test)]\n#[allow(dead_code)]\nfn helper(x: [u8; 2]) -> u8 { let _ = std::env::args(); 0 }\n"
            ),
            Vec::<&str>::new()
        );
        assert_eq!(apis("#![cfg(test)]\nuse std::fs;\n"), Vec::<&str>::new());
    }

    #[test]
    fn a_forbidden_api_after_a_test_module_is_found_and_cannot_hide_behind_ordering() {
        let source = "#[cfg(test)]\nmod tests {\n    fn t() { if true { } }\n}\n\nuse std::fs;\n";
        assert_eq!(apis(source), ["std::fs"]);
        let two = "#[cfg(test)] mod a { use std::net::X; }\nfn p() { std::thread::spawn(f); }\n#[cfg(test)] mod b {}\nuse std::env;\n";
        assert_eq!(apis(two), ["std::thread", "std::env"]);
        // `cfg(not(test))` is production code.
        assert_eq!(apis("#[cfg(not(test))]\nuse std::os::raw;\n"), ["std::os"]);
        // A test attribute on a body-less item does not swallow the code after it.
        assert_eq!(
            apis("#[cfg(test)]\nmod tests;\nuse std::fs;\n"),
            ["std::fs"]
        );
    }

    #[test]
    fn an_inner_test_attribute_exempts_only_its_own_module() {
        // The inner attribute belongs to the inline module, not to the file.
        assert_eq!(
            apis(
                "mod tests {\n    #![cfg(test)]\n    use std::process::Command;\n}\nuse std::fs;\n"
            ),
            ["std::fs"]
        );
        assert_eq!(
            apis("mod a { #![cfg(test)] use std::net::X; }\nmod b { use std::env; }\n"),
            ["std::env"]
        );
        assert_eq!(
            apis("mod outer { mod tests { #![cfg(test)] use std::fs; } use std::env; }\n"),
            ["std::env"]
        );
        // A file-level inner attribute before any item still exempts the whole file.
        assert_eq!(
            apis(
                "//! doc\n#![allow(dead_code)]\n#![cfg(test)]\nuse std::fs;\nmod m { use std::net::X; }\n"
            ),
            Vec::<&str>::new()
        );
        // After an item it is misplaced and exempts nothing.
        assert_eq!(
            apis("use std::fs;\n#![cfg(test)]\nuse std::env;\n"),
            ["std::fs", "std::env"]
        );
    }

    #[test]
    fn grouped_imports_with_aliases_are_forbidden() {
        assert_eq!(apis("use std::{fs as filesystem};\n"), ["std::fs"]);
        assert_eq!(apis("use std::{fs as f, io};\n"), ["std::fs"]);
        assert_eq!(apis("use std::{ io , net as n };\n"), ["std::net"]);
        assert_eq!(
            apis("use std::{\n    io,\n    process as p,\n};\n"),
            ["std::process"]
        );
        assert_eq!(apis("use std::{io, fs::{self as x}};\n"), ["std::fs"]);
        assert_eq!(
            apis("use std::{io::{self as i}, thread};\n"),
            ["std::thread"]
        );
        // An alias that merely looks like a forbidden name is harmless.
        assert_eq!(apis("use std::{io as fs};\n"), Vec::<&str>::new());
    }

    #[test]
    fn comments_and_strings_do_not_count_but_groups_do() {
        assert_eq!(
            apis(
                "// std::fs\n/* std::net /* nested */ std::env */\nlet s = \"std::process\";\nlet r = r#\"std::os \"quoted\"\"#;\n"
            ),
            Vec::<&str>::new()
        );
        assert_eq!(apis("use std::{fs, io};\n"), ["std::fs"]);
        assert_eq!(
            apis("use std::{io::{self, Read}, thread};\n"),
            ["std::thread"]
        );
        assert_eq!(
            apis("use std::{io, collections::HashMap};\n"),
            Vec::<&str>::new()
        );
        assert_eq!(apis("fn f<'a>(x: &'a str) { use std::fs; }\n"), ["std::fs"]);
    }

    const CI_OK: &str = "\
jobs:
  test:
    steps:
      - run: cargo test
  wasm:
    steps:
      - uses: setup
        with:
          targets: wasm32-unknown-unknown
      - run: |
          list=\"$(cargo run --locked --quiet -p beamcraft-boundaries -- portable)\"
          cargo check --target wasm32-unknown-unknown $list
";

    fn ci_with(extra: &str) -> String {
        format!("{CI_OK}{extra}")
    }

    #[test]
    fn ci_deriving_the_list_from_the_tool_passes() {
        assert_eq!(check_ci(CI_OK), []);
    }

    #[test]
    fn ci_without_a_wasm_job_or_portable_call_fails() {
        assert_eq!(
            check_ci("jobs:\n  test:\n    steps:\n      - run: cargo test -p beamcraft-geom\n"),
            [Violation::CiNoWasmJob]
        );
        let no_call = CI_OK.replace(
            "beamcraft-boundaries -- portable",
            "beamcraft-boundaries -- check",
        );
        assert_eq!(check_ci(&no_call), [Violation::CiMissingPortableCall]);
    }

    #[test]
    fn ci_rejects_a_hand_written_list_on_the_same_line() {
        let v = check_ci(&ci_with(
            "      - run: cargo check --target wasm32-unknown-unknown -p beamcraft-geom\n",
        ));
        assert!(
            matches!(v.as_slice(), [Violation::CiHardCodedList(l)] if l.contains("beamcraft-geom")),
            "{v:?}"
        );
    }

    #[test]
    fn ci_rejects_a_hand_written_list_in_a_continuation_line() {
        let v = check_ci(&ci_with(
            "      - run: |\n          cargo check --target wasm32-unknown-unknown \\\n            -p beamcraft-geom\n",
        ));
        assert!(
            matches!(v.as_slice(), [Violation::CiHardCodedList(l)] if l.contains("-p beamcraft-geom")),
            "{v:?}"
        );
    }

    #[test]
    fn ci_rejects_a_hand_written_list_in_a_variable_assignment() {
        for assignment in [
            "PKGS=\"-p beamcraft-geom\"",
            "PKGS='--package beamcraft-model'",
            "PKGS=-pbeamcraft-geom",
            "PKGS=--package=beamcraft-geom",
        ] {
            let v = check_ci(&ci_with(&format!(
                "      - run: {assignment}\n      - run: cargo check --target wasm32-unknown-unknown $PKGS\n"
            )));
            assert!(
                matches!(v.as_slice(), [Violation::CiHardCodedList(_)]),
                "{assignment}: {v:?}"
            );
        }
    }

    #[test]
    fn ci_rejects_a_hand_written_list_in_a_separate_run_line_without_the_target() {
        let v = check_ci(&ci_with("      - run: cargo check -p beamcraft-model\n"));
        assert!(
            matches!(v.as_slice(), [Violation::CiHardCodedList(_)]),
            "{v:?}"
        );
        // The tool's own `-p` is allowed only on a `cargo run` line.
        let v = check_ci(&ci_with(
            "      - run: cargo check -p beamcraft-boundaries\n",
        ));
        assert!(
            matches!(v.as_slice(), [Violation::CiHardCodedList(_)]),
            "{v:?}"
        );
    }

    #[test]
    fn ci_ignores_package_arguments_of_other_jobs_and_comments() {
        let other = CI_OK.replace(
            "      - run: cargo test\n",
            "      - run: cargo test -p beamcraft-geom\n",
        );
        assert_eq!(check_ci(&other), []);
        assert_eq!(
            check_ci(&ci_with("      # cargo check -p beamcraft-geom\n")),
            []
        );
    }

    #[test]
    fn every_violation_has_a_readable_message() {
        let v = with(good(), |ws| {
            pkg_mut(ws, "model").dependencies.push(Dependency {
                name: "tokio".into(),
                section: "dependencies".into(),
            });
        });
        let text = v
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains("`model`") && text.contains("`tokio`"),
            "{text}"
        );
    }
}
