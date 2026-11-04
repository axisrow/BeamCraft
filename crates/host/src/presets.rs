// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The V07 preset file format: the named discipline presets between engine types and the
//! bounded, versioned JSON document the CLI and the window persist as personal configuration
//! (`view-presets.json` in the per-user configuration directory). Mapping only — every
//! admission rule lives in `beamcraft_engine::presets`. Presets are store-independent display
//! state, so unlike the views layer this file holds no camera, no provenance and no model
//! revision, and it never enters a store.

use beamcraft_engine::element::ElementKind;
use beamcraft_engine::presentation::Representation;
use beamcraft_engine::presets::{PresetBook, ViewPreset};
use beamcraft_json::Json;
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;

/// The format version this module writes; files of any other version refuse whole.
pub const VERSION: u64 = 1;
/// The file holds at most this many bytes.
pub const MAX_BYTES: usize = 65_536;

fn unread(field: &'static str) -> CommandError {
    CommandError::new(
        ErrorCode::InvalidArgument,
        field,
        "preset file field is unreadable",
    )
}

/// A named colour set as JSON: present text or null.
fn colour_set_to_json(set: Option<&str>) -> Json {
    match set {
        Some(name) => Json::text(name),
        None => Json::Null,
    }
}

/// One preset as JSON, in the preset file's element shape.
///
/// # Errors
/// [`ErrorCode::NonFiniteNumber`] when a depth distance is not a JSON number.
pub fn preset_to_json(preset: &ViewPreset) -> Result<Json, CommandError> {
    let distances = preset
        .depth
        .iter()
        .flatten()
        .map(|v| Json::number(*v).map_err(|_| unread("depth_mm")))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json::object([
        ("name", Json::text(&preset.name)),
        (
            "hidden_kinds",
            Json::Array(
                preset
                    .hidden_kinds
                    .hidden_kinds()
                    .into_iter()
                    .map(|k| Json::text(k.as_token()))
                    .collect(),
            ),
        ),
        (
            "depth_mm",
            match preset.depth {
                Some(_) => Json::Array(distances),
                None => Json::Null,
            },
        ),
        (
            "representation",
            Json::text(preset.representation.as_token()),
        ),
        (
            "colour_set",
            colour_set_to_json(preset.colour_set.as_deref()),
        ),
        ("grids_visible", Json::Bool(preset.grids_visible)),
    ]))
}

/// Reads one preset; every key is required, `depth_mm` and `colour_set` may be null.
pub fn preset_from_json(json: &Json) -> Result<ViewPreset, CommandError> {
    let text = |key: &'static str| {
        json.get(key)
            .and_then(Json::string)
            .map(str::to_owned)
            .map_err(|_| unread(key))
    };
    let mut preset = ViewPreset::new(&text("name")?);
    for token in json
        .get("hidden_kinds")
        .and_then(Json::array)
        .map_err(|_| unread("hidden_kinds"))?
    {
        let kind = token
            .string()
            .ok()
            .and_then(ElementKind::parse)
            .ok_or_else(|| unread("hidden_kinds"))?;
        preset.hidden_kinds = preset.hidden_kinds.hiding(kind);
    }
    preset.depth = match json
        .get("depth_mm")
        .ok()
        .filter(|v| !matches!(v, Json::Null))
    {
        Some(_) => {
            let values = json
                .get("depth_mm")
                .and_then(Json::array)
                .map_err(|_| unread("depth_mm"))?
                .iter()
                .map(|v| v.float().map_err(|_| unread("depth_mm")))
                .collect::<Result<Vec<_>, _>>()?;
            Some(<[f64; 2]>::try_from(values).map_err(|_| unread("depth_mm"))?)
        }
        None => None,
    };
    preset.representation =
        Representation::parse(&text("representation")?).ok_or_else(|| unread("representation"))?;
    preset.colour_set = match json
        .get("colour_set")
        .ok()
        .filter(|v| !matches!(v, Json::Null))
    {
        Some(_) => Some(text("colour_set")?),
        None => None,
    };
    preset.grids_visible = json
        .get("grids_visible")
        .and_then(Json::boolean)
        .map_err(|_| unread("grids_visible"))?;
    preset.validate()?;
    Ok(preset)
}

/// The preset file as JSON.
///
/// # Errors
/// [`ErrorCode::NonFiniteNumber`] when a preset carries a non-finite depth distance.
pub fn book_to_json(book: &PresetBook) -> Result<Json, CommandError> {
    let presets = book
        .presets()
        .iter()
        .map(preset_to_json)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json::object([
        ("version", Json::uint(VERSION)),
        ("presets", Json::Array(presets)),
    ]))
}

/// Reads a book from the preset file's JSON; anything malformed, another version, or past a
/// limit refuses whole.
///
/// # Errors
/// [`ErrorCode::InvalidArgument`] for a file of another version or a malformed entry,
/// [`ErrorCode::LimitExceeded`] past the preset count.
pub fn book_from_json(json: &Json) -> Result<PresetBook, CommandError> {
    let version = json
        .get("version")
        .and_then(Json::unsigned)
        .map_err(|_| unread("version"))?;
    if version != VERSION {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "presets.version",
            "the preset file has another format version",
        ));
    }
    let list = json
        .get("presets")
        .and_then(Json::array)
        .map_err(|_| unread("presets"))?;
    if list.len() > beamcraft_engine::presets::MAX_PRESETS {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "presets.count",
            "the preset file exceeds the preset limit",
        ));
    }
    let presets = list
        .iter()
        .map(preset_from_json)
        .collect::<Result<Vec<_>, _>>()?;
    PresetBook::from_parts(presets)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use beamcraft_engine::element::KindFilter;
    use beamcraft_engine::presets::MAX_PRESETS;

    fn preset(name: &str) -> ViewPreset {
        let mut p = ViewPreset::new(name);
        p.hidden_kinds = KindFilter::SHOW_ALL.hiding(ElementKind::Reference);
        p.depth = Some([0.0, 3000.0]);
        p.representation = Representation::Simplified;
        p.colour_set = Some("Part classes".into());
        p.grids_visible = false;
        p
    }

    #[test]
    fn the_book_round_trips_through_json() {
        let mut book = PresetBook::new();
        book.insert(preset("Fabrication")).unwrap();
        book.insert(ViewPreset::new("Erection")).unwrap();
        let back = book_from_json(&book_to_json(&book).unwrap()).unwrap();
        assert_eq!(back, book);
        assert_eq!(
            back.find("fabrication").map(|p| p.name.as_str()),
            Some("Fabrication")
        );
    }

    #[test]
    fn malformed_files_and_other_versions_refuse_whole() {
        let good = book_to_json(&PresetBook::new()).unwrap();
        assert!(book_from_json(&good).unwrap().is_empty());
        let wrong_version = Json::object([("version", Json::uint(VERSION + 1))]);
        assert!(book_from_json(&wrong_version).is_err());
        let mut broken = preset_to_json(&preset("A")).unwrap();
        if let Json::Object(map) = &mut broken {
            map.insert("representation".into(), Json::text("axis"));
        }
        let file = Json::object([
            ("version", Json::uint(VERSION)),
            ("presets", Json::Array(vec![broken])),
        ]);
        assert!(book_from_json(&file).is_err());
        let twice = Json::object([
            ("version", Json::uint(VERSION)),
            (
                "presets",
                Json::Array(vec![
                    preset_to_json(&preset("a")).unwrap(),
                    preset_to_json(&preset("A")).unwrap(),
                ]),
            ),
        ]);
        assert!(book_from_json(&twice).is_err(), "names stay unique");
        let over = Json::object([
            ("version", Json::uint(VERSION)),
            (
                "presets",
                Json::Array(
                    (0..=MAX_PRESETS)
                        .map(|n| preset_to_json(&ViewPreset::new(&format!("p{n}"))).unwrap())
                        .collect(),
                ),
            ),
        ]);
        assert_eq!(
            book_from_json(&over).unwrap_err().code,
            ErrorCode::LimitExceeded
        );
    }
}
