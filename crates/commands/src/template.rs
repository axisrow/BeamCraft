// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The starter content of an environment: the grids and levels a new project of that
//! environment is seeded with (#840).
//!
//! An environment may declare `bim:environment:template`, a contained relative path to a USD
//! layer of template entries. Each entry is a prim `/Template/<id>` with `steel:template:kind`
//! (token: `level` or `grid`) and `steel:template:name` (string). A level adds
//! `steel:template:elevationMm` (double). A grid adds the spacing of its two families in the
//! words of `create-grid`: `steel:template:uExpression`, `uLabels`, `vExpression`, `vLabels`
//! (strings, an expression is required for at least one family) and optional
//! `steel:template:origin` (string `X,Y` in millimetres) and `steel:template:rotationDeg`
//! (double). Entries are applied in id order.
//!
//! Unlike presets, a template is all or nothing: seeding half of a starter layout would hand the
//! person a project that looks complete and is not, so one refused entry refuses the template.
//! Reading a template never changes a model; the file is read under the foreign-layer budgets.

use crate::definition::field;
use crate::library::read_all;
use crate::plate::{number, text};
use crate::source_file::read_file;
use beamcraft_layer_engine::{Prim, Value};
use beamcraft_model::command::label_is_valid;
use beamcraft_model::error::ErrorCode;
use session::Refusal;
use session::environment::{MAX_ENVIRONMENT_BYTES, parse_environment};
use session::foreign;
use std::path::Path;

/// Largest template file, in bytes.
pub const MAX_TEMPLATE_BYTES: usize = 1024 * 1024;
/// Most entries one template may hold; more is `LIMIT_EXCEEDED`.
pub const MAX_TEMPLATE_ENTRIES: usize = 64;

const PARENT: &str = "/Template";

/// A starter grid, in the option words of `create-grid`.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateGrid {
    /// The grid name.
    pub name: String,
    /// The spacing expression of the U family (lines across X), when it has one.
    pub u_expression: Option<String>,
    /// The labels of the U family, when typed.
    pub u_labels: Option<String>,
    /// The spacing expression of the V family (lines across Y), when it has one.
    pub v_expression: Option<String>,
    /// The labels of the V family, when typed.
    pub v_labels: Option<String>,
    /// The origin `X,Y` in millimetres, when not the project origin.
    pub origin: Option<String>,
    /// The rotation about Z in degrees, when not zero.
    pub rotation_deg: Option<f64>,
}

/// One admitted template entry.
#[derive(Debug, Clone, PartialEq)]
pub enum TemplateEntry {
    /// A level: its name and elevation in millimetres.
    Level {
        /// The level name.
        name: String,
        /// Elevation in millimetres.
        elevation_mm: f64,
    },
    /// A grid.
    Grid(TemplateGrid),
}

/// The entries of one template, in id order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Template {
    /// Admitted entries in id order.
    pub entries: Vec<TemplateEntry>,
}

fn typed<'a>(prim: &'a Prim, name: &str, ty: &str) -> Result<Option<&'a Value>, &'static str> {
    field(prim, name, ty).map_err(|_| "a template attribute has another type")
}

fn string(prim: &Prim, name: &str) -> Result<Option<String>, &'static str> {
    match typed(prim, name, "string")? {
        None => Ok(None),
        Some(value) => text(value)
            .filter(|t| label_is_valid(t))
            .map(|t| Some(t.to_owned()))
            .ok_or("a template text is not a valid label"),
    }
}

fn finite(prim: &Prim, name: &str) -> Result<Option<f64>, &'static str> {
    typed(prim, name, "double")?
        .map(|v| {
            number(v)
                .filter(|n| n.is_finite())
                .ok_or("a template number is not finite")
        })
        .transpose()
}

fn decode(prim: &Prim) -> Result<TemplateEntry, &'static str> {
    let name = string(prim, "steel:template:name")?.ok_or("the template entry has no name")?;
    match typed(prim, "steel:template:kind", "token")?.and_then(text) {
        Some("level") => Ok(TemplateEntry::Level {
            name,
            elevation_mm: finite(prim, "steel:template:elevationMm")?
                .ok_or("a template level has no elevation")?,
        }),
        Some("grid") => {
            let grid = TemplateGrid {
                name,
                u_expression: string(prim, "steel:template:uExpression")?,
                u_labels: string(prim, "steel:template:uLabels")?,
                v_expression: string(prim, "steel:template:vExpression")?,
                v_labels: string(prim, "steel:template:vLabels")?,
                origin: string(prim, "steel:template:origin")?,
                rotation_deg: finite(prim, "steel:template:rotationDeg")?,
            };
            if grid.u_expression.is_none() && grid.v_expression.is_none() {
                return Err("a template grid has no spacing expression");
            }
            Ok(TemplateEntry::Grid(grid))
        }
        _ => Err("the template kind is missing or unknown"),
    }
}

/// Reads the entries of a template layer's bytes.
///
/// # Errors
/// `LIMIT_EXCEEDED` for a file over 1 MiB or more than 64 entries, `INVALID_ARGUMENT` for bytes
/// that are not a self-contained USD layer in millimetres, Z up, or for any entry that is not
/// admitted (the whole template is refused).
pub fn load_bytes(bytes: Vec<u8>) -> Result<Template, Refusal> {
    if bytes.len() > MAX_TEMPLATE_BYTES {
        return Err(Refusal::new(
            ErrorCode::LimitExceeded,
            "the template file is larger than 1 MiB",
        ));
    }
    let layer = foreign::open(
        bytes,
        "the template is not a self-contained USD layer in millimetres, Z up",
    )?;
    let prims = read_all(&layer.reader, &layer.opened, &layer.control)?;
    let mut entries = Vec::new();
    for prim in &prims {
        let Some((parent, id)) = prim.path.rsplit_once('/') else {
            continue;
        };
        if parent != PARENT || !prim.active {
            continue;
        }
        if entries.len() == MAX_TEMPLATE_ENTRIES {
            return Err(Refusal::new(
                ErrorCode::LimitExceeded,
                "the template has more than 64 entries",
            ));
        }
        let entry = decode(prim).map_err(Refusal::invalid)?;
        entries.push((id.to_owned(), entry));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(Template {
        entries: entries.into_iter().map(|(_, entry)| entry).collect(),
    })
}

/// The template of the environment file `environment`: empty when it declares none.
///
/// # Errors
/// A typed [`Refusal`] when the environment or its template cannot be read or admitted.
pub fn load_environment(environment: &Path) -> Result<Template, Refusal> {
    let unreadable = |code| match code {
        ErrorCode::NotFound => Refusal::new(code, "the template file could not be opened"),
        ErrorCode::LimitExceeded => Refusal::new(code, "the file is over its size limit"),
        _ => Refusal::invalid("the environment path is not a readable regular local file"),
    };
    let path = environment
        .to_str()
        .ok_or_else(|| Refusal::invalid("the environment path is not UTF-8"))?;
    let info = parse_environment(read_file(path, MAX_ENVIRONMENT_BYTES).map_err(unreadable)?)?;
    let Some(reference) = info.template else {
        return Ok(Template::default());
    };
    let file = environment
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(reference);
    let file = file
        .to_str()
        .ok_or_else(|| Refusal::invalid("the template path is not UTF-8"))?;
    load_bytes(read_file(file, MAX_TEMPLATE_BYTES).map_err(unreadable)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(body: &str) -> Vec<u8> {
        format!(
            "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n)\n\ndef Scope \"Template\"\n{{\n{body}\n}}\n"
        )
        .into_bytes()
    }

    #[test]
    fn levels_and_grids_are_read_in_id_order() {
        let template = load_bytes(layer(
            "    def Scope \"B_Grid\"\n    {\n        token steel:template:kind = \"grid\"\n        string steel:template:name = \"Grid\"\n        string steel:template:uExpression = \"0 2*6000\"\n        string steel:template:uLabels = \"1 2 3\"\n        string steel:template:vExpression = \"0 2*6000\"\n        string steel:template:vLabels = \"A B C\"\n    }\n    def Scope \"A_Level\"\n    {\n        token steel:template:kind = \"level\"\n        string steel:template:name = \"+0.000\"\n        double steel:template:elevationMm = 0\n    }\n",
        ))
        .unwrap();
        assert_eq!(template.entries.len(), 2);
        assert_eq!(
            template.entries[0],
            TemplateEntry::Level {
                name: "+0.000".into(),
                elevation_mm: 0.0
            }
        );
        let TemplateEntry::Grid(grid) = &template.entries[1] else {
            panic!("the second entry is the grid");
        };
        assert_eq!(grid.u_expression.as_deref(), Some("0 2*6000"));
        assert_eq!(grid.v_labels.as_deref(), Some("A B C"));
        assert_eq!(grid.origin, None);
    }

    #[test]
    fn one_bad_entry_refuses_the_whole_template() {
        for body in [
            "    def Scope \"X\"\n    {\n        token steel:template:kind = \"roof\"\n        string steel:template:name = \"R\"\n    }\n",
            "    def Scope \"X\"\n    {\n        token steel:template:kind = \"level\"\n        string steel:template:name = \"L\"\n    }\n",
            "    def Scope \"X\"\n    {\n        token steel:template:kind = \"grid\"\n        string steel:template:name = \"G\"\n    }\n",
            "    def Scope \"X\"\n    {\n        token steel:template:kind = \"level\"\n        double steel:template:elevationMm = 0\n    }\n",
        ] {
            let error = load_bytes(layer(body)).unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidArgument, "{body}");
        }
    }

    #[test]
    fn an_oversized_template_is_refused() {
        let error = load_bytes(vec![b' '; MAX_TEMPLATE_BYTES + 1]).unwrap_err();
        assert_eq!(error.code, ErrorCode::LimitExceeded);
        let many: String = (0..=MAX_TEMPLATE_ENTRIES)
            .map(|i| {
                format!(
                    "    def Scope \"L{i:03}\"\n    {{\n        token steel:template:kind = \"level\"\n        string steel:template:name = \"L{i}\"\n        double steel:template:elevationMm = {i}\n    }}\n"
                )
            })
            .collect();
        let error = load_bytes(layer(&many)).unwrap_err();
        assert_eq!(error.code, ErrorCode::LimitExceeded);
    }

    #[test]
    fn the_shipped_blank_template_seeds_one_grid_and_one_level() {
        let file =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../environments/blank/environment.usda");
        let template = load_environment(&file).unwrap();
        let levels = template
            .entries
            .iter()
            .filter(|e| matches!(e, TemplateEntry::Level { .. }))
            .count();
        let grids = template
            .entries
            .iter()
            .filter(|e| matches!(e, TemplateEntry::Grid(_)))
            .count();
        assert_eq!((grids, levels), (1, 1));
    }
}
