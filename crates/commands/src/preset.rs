// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Part presets of an environment: the defaults a new beam, column or plate starts from.
//!
//! An environment may declare `bim:environment:presets`, a contained relative path to a USD layer
//! of preset entries. Each entry is a prim `/Presets/<id>` with `steel:preset:kind` (token:
//! `beam`, `column` or `plate`), `steel:preset:name` (string), `steel:preset:class` (int) and
//! optional `steel:preset:material` (a material display name), `steel:preset:profile` (a profile
//! designation, members only) and `steel:preset:thicknessMm` (double, plates only). The first
//! entry of a kind in id order is its default. Presets are inputs like libraries: reading one
//! never changes a model, and the file is read under the foreign-layer budgets. A refused entry
//! is reported and skipped, never silently dropped.

use crate::definition::field;
use crate::library::read_all;
use crate::plate::{number, text};
use crate::project::project_environment_file;
use crate::source_file::read_file;
use beamcraft_layer_engine::{Prim, Value};
use beamcraft_model::command::label_is_valid;
use beamcraft_model::error::ErrorCode;
use session::Refusal;
use session::environment::{MAX_ENVIRONMENT_BYTES, parse_environment};
use session::foreign;
use std::path::Path;

/// Largest presets file, in bytes.
pub const MAX_PRESETS_BYTES: usize = 1024 * 1024;
/// Most presets one file may hold; more is `LIMIT_EXCEEDED`.
pub const MAX_PRESETS: usize = 256;

const PARENT: &str = "/Presets";

/// The part a preset is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PresetKind {
    /// A straight member placed as a beam.
    Beam,
    /// A straight member placed as a column.
    Column,
    /// A plate.
    Plate,
}

impl PresetKind {
    /// The token written in `steel:preset:kind` and in the `definitions` reply.
    pub fn token(self) -> &'static str {
        match self {
            Self::Beam => "beam",
            Self::Column => "column",
            Self::Plate => "plate",
        }
    }

    fn from_token(token: &str) -> Option<Self> {
        [Self::Beam, Self::Column, Self::Plate]
            .into_iter()
            .find(|kind| kind.token() == token)
    }
}

/// One admitted preset.
#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    /// The prim name, unique in its file.
    pub id: String,
    /// The part it is for.
    pub kind: PresetKind,
    /// The default part name.
    pub name: String,
    /// The default part class.
    pub class: i32,
    /// The display name of the default material, when it has one.
    pub material: Option<String>,
    /// The designation of the default profile (members only).
    pub profile: Option<String>,
    /// The default thickness in millimetres (plates only).
    pub thickness_mm: Option<f64>,
}

/// An entry that was read but refused; the other entries still load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedPreset {
    /// USD path of the entry prim.
    pub prim_path: String,
    /// Fixed explanation.
    pub message: &'static str,
}

/// The presets of one file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Presets {
    /// Admitted presets in id order.
    pub presets: Vec<Preset>,
    /// Refused entries in path order.
    pub rejected: Vec<RejectedPreset>,
}

impl Presets {
    /// The default preset of `kind`: the first one in id order.
    pub fn default_for(&self, kind: PresetKind) -> Option<&Preset> {
        self.presets.iter().find(|p| p.kind == kind)
    }
}

fn integer(value: &Value) -> Option<i32> {
    match value {
        Value::Int(n) => i32::try_from(*n).ok(),
        Value::Uint(n) => i32::try_from(*n).ok(),
        _ => None,
    }
}

fn typed<'a>(prim: &'a Prim, name: &str, ty: &str) -> Result<Option<&'a Value>, &'static str> {
    field(prim, name, ty).map_err(|_| "a preset attribute has another type")
}

fn label(prim: &Prim, name: &str) -> Result<Option<String>, &'static str> {
    match typed(prim, name, "string")? {
        None => Ok(None),
        Some(value) => text(value)
            .filter(|t| label_is_valid(t))
            .map(|t| Some(t.to_owned()))
            .ok_or("a preset text is not a valid label"),
    }
}

fn decode(prim: &Prim, id: &str) -> Result<Preset, &'static str> {
    let kind = typed(prim, "steel:preset:kind", "token")?
        .and_then(text)
        .and_then(PresetKind::from_token)
        .ok_or("the preset kind is missing or unknown")?;
    let name = label(prim, "steel:preset:name")?.ok_or("the preset has no name")?;
    let class = match typed(prim, "steel:preset:class", "int")? {
        None => 0,
        Some(value) => integer(value).ok_or("the preset class is not an int32")?,
    };
    let thickness_mm = typed(prim, "steel:preset:thicknessMm", "double")?
        .map(|v| {
            number(v)
                .filter(|t| t.is_finite() && *t > 0.0)
                .ok_or("the preset thickness is not a positive number")
        })
        .transpose()?;
    let profile = label(prim, "steel:preset:profile")?;
    if kind == PresetKind::Plate && profile.is_some() {
        return Err("a plate preset has no profile");
    }
    if kind != PresetKind::Plate && thickness_mm.is_some() {
        return Err("only a plate preset has a thickness");
    }
    Ok(Preset {
        id: id.to_owned(),
        kind,
        name,
        class,
        material: label(prim, "steel:preset:material")?,
        profile,
        thickness_mm,
    })
}

/// Reads the presets of a layer's bytes.
///
/// # Errors
/// `LIMIT_EXCEEDED` for a file over 1 MiB or more than 256 entries, `INVALID_ARGUMENT` for bytes
/// that are not a self-contained USD layer in millimetres, Z up.
pub fn load_bytes(bytes: Vec<u8>) -> Result<Presets, Refusal> {
    if bytes.len() > MAX_PRESETS_BYTES {
        return Err(Refusal::new(
            ErrorCode::LimitExceeded,
            "the presets file is larger than 1 MiB",
        ));
    }
    let layer = foreign::open(
        bytes,
        "the presets are not a self-contained USD layer in millimetres, Z up",
    )?;
    let prims = read_all(&layer.reader, &layer.opened, &layer.control)?;
    let mut out = Presets::default();
    let mut count = 0usize;
    for prim in &prims {
        let Some((parent, id)) = prim.path.rsplit_once('/') else {
            continue;
        };
        if parent != PARENT {
            continue;
        }
        count += 1;
        if count > MAX_PRESETS {
            return Err(Refusal::new(
                ErrorCode::LimitExceeded,
                "the presets file has more than 256 entries",
            ));
        }
        let decoded = if prim.active {
            decode(prim, id)
        } else {
            Err("the preset is inactive")
        };
        match decoded {
            Ok(preset) => out.presets.push(preset),
            Err(message) => out.rejected.push(RejectedPreset {
                prim_path: prim.path.clone(),
                message,
            }),
        }
    }
    out.presets.sort_by(|a, b| a.id.cmp(&b.id));
    out.rejected.sort_by(|a, b| a.prim_path.cmp(&b.prim_path));
    Ok(out)
}

/// The presets of the environment copied into the project at `dir`. A project without an
/// environment, or an environment that declares no presets, has none.
///
/// # Errors
/// A typed [`Refusal`] when the environment or its presets file cannot be read or admitted.
pub fn load_project(dir: &Path) -> Result<Presets, Refusal> {
    let Some(file) = project_environment_file(dir) else {
        return Ok(Presets::default());
    };
    let unreadable = |code| match code {
        ErrorCode::NotFound => Refusal::new(code, "the presets file could not be opened"),
        ErrorCode::LimitExceeded => Refusal::new(code, "the file is over its size limit"),
        _ => Refusal::invalid("the environment path is not a readable regular local file"),
    };
    let path = file
        .to_str()
        .ok_or_else(|| Refusal::invalid("the project environment path is not UTF-8"))?;
    let info = parse_environment(read_file(path, MAX_ENVIRONMENT_BYTES).map_err(unreadable)?)?;
    let Some(reference) = info.presets else {
        return Ok(Presets::default());
    };
    let presets = file
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(reference);
    let presets = presets
        .to_str()
        .ok_or_else(|| Refusal::invalid("the presets path is not UTF-8"))?;
    load_bytes(read_file(presets, MAX_PRESETS_BYTES).map_err(unreadable)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(body: &str) -> Vec<u8> {
        format!(
            "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n)\n\ndef Scope \"Presets\"\n{{\n{body}\n}}\n"
        )
        .into_bytes()
    }

    #[test]
    fn presets_are_read_by_kind_and_the_first_of_a_kind_is_the_default() {
        let bytes = layer(
            r#"
    def Scope "B_Second" { token steel:preset:kind = "beam"
        string steel:preset:name = "Other" }
    def Scope "A_Beam" { token steel:preset:kind = "beam"
        string steel:preset:name = "Beam"
        int steel:preset:class = 3
        string steel:preset:material = "Steel"
        string steel:preset:profile = "I-300x150" }
    def Scope "P_Plate" { token steel:preset:kind = "plate"
        string steel:preset:name = "Plate"
        double steel:preset:thicknessMm = 12 }
"#,
        );
        let loaded = load_bytes(bytes).unwrap();
        assert!(loaded.rejected.is_empty(), "{:?}", loaded.rejected);
        let beam = loaded.default_for(PresetKind::Beam).unwrap();
        assert_eq!((beam.name.as_str(), beam.class), ("Beam", 3));
        assert_eq!(beam.material.as_deref(), Some("Steel"));
        assert_eq!(beam.profile.as_deref(), Some("I-300x150"));
        let plate = loaded.default_for(PresetKind::Plate).unwrap();
        assert_eq!(plate.thickness_mm, Some(12.0));
        assert!(loaded.default_for(PresetKind::Column).is_none());
    }

    #[test]
    fn a_bad_entry_is_reported_and_the_others_load() {
        let bytes = layer(
            r#"
    def Scope "Ok" { token steel:preset:kind = "column"
        string steel:preset:name = "Column" }
    def Scope "BadKind" { token steel:preset:kind = "wall"
        string steel:preset:name = "Wall" }
    def Scope "ThickBeam" { token steel:preset:kind = "beam"
        string steel:preset:name = "Beam"
        double steel:preset:thicknessMm = 5 }
    def Scope "NegativePlate" { token steel:preset:kind = "plate"
        string steel:preset:name = "Plate"
        double steel:preset:thicknessMm = -1 }
"#,
        );
        let loaded = load_bytes(bytes).unwrap();
        assert_eq!(loaded.presets.len(), 1);
        assert_eq!(loaded.rejected.len(), 3);
    }

    #[test]
    fn bytes_that_are_not_a_layer_are_refused() {
        assert_eq!(
            load_bytes(b"not usd".to_vec()).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            load_bytes(vec![b' '; MAX_PRESETS_BYTES + 1])
                .unwrap_err()
                .code,
            ErrorCode::LimitExceeded
        );
    }

    #[test]
    fn the_shipped_presets_name_profiles_the_environment_has() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../environments/blank");
        let file = dir.join("environment.usda");
        let environment = crate::library::load_environment(file.to_str().unwrap()).unwrap();
        let bytes = std::fs::read(dir.join("presets.usda")).unwrap();
        let loaded = load_bytes(bytes).unwrap();
        assert!(loaded.rejected.is_empty(), "{:?}", loaded.rejected);
        for kind in [PresetKind::Beam, PresetKind::Column, PresetKind::Plate] {
            assert!(loaded.default_for(kind).is_some(), "{kind:?}");
        }
        for profile in loaded.presets.iter().filter_map(|p| p.profile.as_deref()) {
            let lookup = crate::library::Lookup::new(environment.sources.clone());
            assert!(lookup.find(profile).is_ok(), "{profile}");
        }
    }

    #[test]
    fn a_project_without_an_environment_has_no_presets() {
        let dir =
            std::env::temp_dir().join(format!("beamcraft-presets-none-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        assert_eq!(load_project(&dir).unwrap(), Presets::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
