// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The module table and the third-party allowlist, read from
//! `docs/contracts/f08-module-boundaries-v0.1.md`.
//!
//! Each is a Markdown table between two HTML-comment markers. The document is the only source;
//! this module never writes it.

use std::fmt;

/// Repository path of the boundary document.
pub const DOCUMENT_PATH: &str = "docs/contracts/f08-module-boundaries-v0.1.md";
/// Largest document accepted, bytes.
pub const MAX_DOCUMENT_BYTES: usize = 1 << 20;
/// Most rows accepted in one table.
pub const MAX_ROWS: usize = 256;

/// What a package is allowed to be and do (document section 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Pure, portable domain crate.
    Domain,
    /// Desktop-native library with no WASM requirement (WS amendment to F08).
    Native,
    /// Native engine host.
    Host,
    /// One discipline (steel, later concrete or MEP): its commands, model types and operations.
    /// Only an app or a tool may depend on it, so a discipline never depends on another and the
    /// platform never depends on a discipline.
    Discipline,
    /// Binary entry point.
    App,
    /// Development tool.
    Tool,
}

impl Kind {
    /// Whether this kind requires WASM-clean source and dependencies.
    ///
    /// F08 reserves `Domain` for geom, model and kernel; all other crates are desktop-native.
    pub const fn requires_wasm(self) -> bool {
        matches!(self, Self::Domain)
    }
}

/// Whether a row is a workspace member today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// A workspace member.
    Now,
    /// Reserved; created by its own task.
    Planned,
}

/// One row of the module table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Cargo package name.
    pub package: String,
    /// Path of the package directory, relative to the workspace root, with `/` separators.
    pub path: String,
    /// Kind of package.
    pub kind: Kind,
    /// Layer number; dependencies must point to a strictly lower layer.
    pub layer: u8,
    /// Internal packages it may depend on.
    pub allowed: Vec<String>,
    /// Present or planned.
    pub status: Status,
    /// Task that owns or creates it.
    pub owner: String,
}

/// The parsed documents.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModuleTable {
    /// Module rows in document order.
    pub rows: Vec<Row>,
    /// Third-party crates that domain crates may use.
    pub third_party: Vec<String>,
}

impl ModuleTable {
    /// The row for a package name.
    pub fn row(&self, package: &str) -> Option<&Row> {
        self.rows.iter().find(|r| r.package == package)
    }

    /// Names of the three `now` domain packages required by the WS amendment to F08.
    pub fn portable(&self) -> Vec<&str> {
        self.rows
            .iter()
            .filter(|r| r.kind.requires_wasm() && r.status == Status::Now)
            .map(|r| r.package.as_str())
            .collect()
    }
}

/// Why the document was not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableError {
    /// The document is larger than [`MAX_DOCUMENT_BYTES`].
    TooLarge,
    /// A marker pair is missing or repeated.
    Markers(&'static str),
    /// A table has more than [`MAX_ROWS`] rows.
    TooManyRows,
    /// A row has the wrong number of cells.
    Cells {
        /// One-based line number inside the table.
        line: usize,
    },
    /// A cell holds an unknown or malformed value.
    Value {
        /// Package of the row.
        package: String,
        /// Column name.
        column: &'static str,
    },
}

impl fmt::Display for TableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => f.write_str("document is larger than the 1 MiB limit"),
            Self::Markers(name) => write!(f, "marker pair `{name}` is missing or repeated"),
            Self::TooManyRows => f.write_str("a table has more than 256 rows"),
            Self::Cells { line } => write!(f, "table line {line} has the wrong number of cells"),
            Self::Value { package, column } => {
                write!(f, "row `{package}` has an invalid `{column}` value")
            }
        }
    }
}

impl std::error::Error for TableError {}

/// The text strictly between `<!-- name:begin -->` and `<!-- name:end -->`.
fn region<'a>(document: &'a str, name: &'static str) -> Result<&'a str, TableError> {
    let begin = format!("<!-- {name}:begin -->");
    let end = format!("<!-- {name}:end -->");
    if document.matches(&begin).count() != 1 || document.matches(&end).count() != 1 {
        return Err(TableError::Markers(name));
    }
    let (_, after) = document
        .split_once(&begin)
        .ok_or(TableError::Markers(name))?;
    let (inside, _) = after.split_once(&end).ok_or(TableError::Markers(name))?;
    Ok(inside)
}

/// Data rows of a pipe table: the cells of every line after the header and separator.
fn data_rows(text: &str, columns: usize) -> Result<Vec<Vec<String>>, TableError> {
    let mut rows = Vec::new();
    let mut table_lines = text.lines().map(str::trim).filter(|l| l.starts_with('|'));
    table_lines.next(); // header
    table_lines.next(); // separator
    for (n, line) in table_lines.enumerate() {
        if rows.len() >= MAX_ROWS {
            return Err(TableError::TooManyRows);
        }
        let inner = line.trim_start_matches('|').trim_end_matches('|');
        let cells: Vec<String> = inner
            .split('|')
            .map(|c| c.trim().trim_matches('`').trim().to_owned())
            .collect();
        if cells.len() != columns {
            return Err(TableError::Cells {
                line: n.saturating_add(3),
            });
        }
        rows.push(cells);
    }
    Ok(rows)
}

fn kind(value: &str) -> Option<Kind> {
    match value {
        "domain" => Some(Kind::Domain),
        "native" => Some(Kind::Native),
        "host" => Some(Kind::Host),
        "discipline" => Some(Kind::Discipline),
        "app" => Some(Kind::App),
        "tool" => Some(Kind::Tool),
        _ => None,
    }
}

fn status(value: &str) -> Option<Status> {
    match value {
        "now" => Some(Status::Now),
        "planned" => Some(Status::Planned),
        _ => None,
    }
}

/// Reads the module table and the third-party allowlist from the document text.
///
/// # Errors
///
/// [`TableError`] if a marker pair is missing, a row is malformed or a size limit is exceeded.
pub fn parse_document(document: &str) -> Result<ModuleTable, TableError> {
    if document.len() > MAX_DOCUMENT_BYTES {
        return Err(TableError::TooLarge);
    }
    let mut table = ModuleTable::default();
    for cells in data_rows(region(document, "module-table")?, 7)? {
        let mut cell = cells.into_iter();
        let (
            Some(package),
            Some(path),
            Some(kind_text),
            Some(layer),
            Some(allowed),
            Some(state),
            Some(owner),
        ) = (
            cell.next(),
            cell.next(),
            cell.next(),
            cell.next(),
            cell.next(),
            cell.next(),
            cell.next(),
        )
        else {
            return Err(TableError::Cells { line: 0 });
        };
        let bad = |column| TableError::Value {
            package: package.clone(),
            column,
        };
        let allowed = if allowed == "-" {
            Vec::new()
        } else {
            allowed.split(',').map(|a| a.trim().to_owned()).collect()
        };
        table.rows.push(Row {
            kind: kind(&kind_text).ok_or_else(|| bad("Kind"))?,
            layer: layer.parse().map_err(|_| bad("Layer"))?,
            status: status(&state).ok_or_else(|| bad("Status"))?,
            path,
            allowed,
            owner,
            package: package.clone(),
        });
    }
    table.third_party = data_rows(region(document, "third-party")?, 2)?
        .into_iter()
        .filter_map(|cells| cells.into_iter().next())
        .collect();
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "intro\n<!-- module-table:begin -->\n\
| Package | Path | Kind | Layer | Allowed dependencies | Status | Owner task |\n\
|---|---|---|---|---|---|---|\n\
| `a` | crates/a | domain | 0 | - | now | G01 |\n\
| b | crates/b | domain | 1 | a | now | P01 |\n\
| c | crates/c | host | 2 | a, b | planned | U01 |\n\
<!-- module-table:end -->\n\
<!-- third-party:begin -->\n| Crate | Reason |\n|---|---|\n| foo | reason |\n<!-- third-party:end -->\n";

    #[test]
    fn reads_rows_and_the_allowlist() {
        let t = parse_document(DOC).unwrap();
        assert_eq!(t.rows.len(), 3);
        assert_eq!(t.row("a").map(|r| r.kind), Some(Kind::Domain));
        assert_eq!(
            t.row("c").map(|r| r.allowed.clone()),
            Some(vec!["a".into(), "b".into()])
        );
        assert_eq!(t.row("c").map(|r| r.status), Some(Status::Planned));
        assert_eq!(t.third_party, ["foo"]);
        assert_eq!(t.portable(), ["a", "b"]);
    }

    #[test]
    fn native_libraries_are_not_wasm_required() {
        let doc = DOC.replace("| b | crates/b | domain |", "| b | crates/b | native |");
        let table = parse_document(&doc).unwrap();
        assert_eq!(table.row("b").map(|r| r.kind), Some(Kind::Native));
        assert_eq!(table.portable(), ["a"]);
        for kind in [
            Kind::Native,
            Kind::Host,
            Kind::Discipline,
            Kind::App,
            Kind::Tool,
        ] {
            assert!(!kind.requires_wasm());
        }
        assert!(Kind::Domain.requires_wasm());
    }

    #[test]
    fn an_empty_allowlist_is_valid() {
        let doc = DOC.replace("| foo | reason |\n", "");
        assert!(parse_document(&doc).unwrap().third_party.is_empty());
    }

    #[test]
    fn rejects_missing_or_repeated_markers() {
        assert_eq!(
            parse_document("nothing"),
            Err(TableError::Markers("module-table"))
        );
        let twice = format!("{DOC}{DOC}");
        assert_eq!(
            parse_document(&twice),
            Err(TableError::Markers("module-table"))
        );
        let no_third = DOC.replace("<!-- third-party:end -->", "");
        assert_eq!(
            parse_document(&no_third),
            Err(TableError::Markers("third-party"))
        );
    }

    #[test]
    fn rejects_bad_cells_and_values() {
        let short = DOC.replace("| a | now | P01 |", "| a | now |");
        assert!(matches!(
            parse_document(&short),
            Err(TableError::Cells { .. })
        ));
        for (from, to, column) in [
            ("| domain | 1 |", "| library | 1 |", "Kind"),
            ("| domain | 1 |", "| domain | x |", "Layer"),
            (
                "| domain | 1 | a | now |",
                "| domain | 1 | a | later |",
                "Status",
            ),
            ("| domain | 1 |", "| domain | 300 |", "Layer"),
        ] {
            let err = parse_document(&DOC.replace(from, to)).unwrap_err();
            assert!(
                matches!(err, TableError::Value { column: c, .. } if c == column),
                "{err}"
            );
        }
    }

    #[test]
    fn rejects_oversized_documents_and_too_many_rows() {
        assert_eq!(
            parse_document(&"x".repeat(MAX_DOCUMENT_BYTES + 1)),
            Err(TableError::TooLarge)
        );
        let row = "| p | crates/p | domain | 0 | - | now | T |\n";
        let doc = DOC.replace(
            "| `a` | crates/a | domain | 0 | - | now | G01 |\n",
            &row.repeat(MAX_ROWS + 1),
        );
        assert_eq!(parse_document(&doc), Err(TableError::TooManyRows));
    }
}
