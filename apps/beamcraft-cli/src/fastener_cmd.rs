// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A14.2 headless command: `fastener-geometry`. It resolves one entry of a
//! `--fastener-library` file, builds the fastener prototype through the engine and prints the
//! per-component quantities of a group of `--count` fasteners: the closed meshes with their
//! digests, the volumes and the mass with its source. It never touches a store.

use crate::report::report;
use beamcraft_commands::definition::DefinitionRecord;
use beamcraft_commands::library::{Lookup, Source, load_fastener};
use beamcraft_json::Json;
use beamcraft_model::ErrorCode;
use session::Refusal;
use std::collections::BTreeMap;
use std::ffi::OsString;
use steel::engine::fastener::{
    ComponentQuantity, FastenerBuild, FastenerSolid, MassSource, prototype, quantities,
};

type Run<T> = Result<T, Refusal>;

/// Runs the fastener geometry command and returns the JSON result and the exit code.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    report(run(args))
}

fn invalid(message: &'static str) -> Refusal {
    Refusal {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message,
    }
}

/// Renders a certified finite quantity; the fallback is never taken for admitted geometry.
fn number(value: f64) -> Json {
    Json::number(value).unwrap_or(Json::Null)
}

fn optional(value: Option<f64>) -> Json {
    value.map_or(Json::Null, number)
}

fn boolean(text: &str) -> Run<bool> {
    match text {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(invalid(
            "--washer-under-head and --washer-under-nut are true or false",
        )),
    }
}

fn approximation_json(solid: &FastenerSolid) -> Json {
    match &solid.approximation {
        None => Json::Null,
        Some(a) => Json::object([
            ("segments", Json::uint(u64::from(a.segments))),
            ("max_sagitta_mm", number(a.max_sagitta_mm)),
            ("sagitta_bound_mm", number(a.sagitta_bound_mm)),
        ]),
    }
}

/// One component row: the prototype mesh facts joined with the group quantities.
fn component_json(row: &ComponentQuantity, build: &FastenerBuild) -> Run<Json> {
    let solid = build
        .components
        .iter()
        .find(|s| s.component == row.component)
        .ok_or_else(|| invalid("the prototype lacks a component of the quantities"))?;
    Ok(Json::object([
        ("component", Json::text(solid.component.token())),
        ("count", Json::uint(u64::from(row.count))),
        ("z_offset_mm", number(solid.z_offset_mm)),
        ("height_mm", number(solid.height_mm)),
        (
            "vertices",
            Json::uint(solid.solid.mesh().vertices().len() as u64),
        ),
        (
            "triangles",
            Json::uint(solid.solid.mesh().triangles().len() as u64),
        ),
        (
            "mesh_digest",
            Json::text(solid.solid.mesh().mesh_digest().to_string()),
        ),
        ("volume_each_mm3", number(row.volume_each_mm3)),
        ("volume_mm3", number(row.volume_mm3)),
        ("mass_kg", optional(row.mass_kg)),
        ("approximation", approximation_json(solid)),
    ]))
}

fn source_json(mass_source: Option<MassSource>) -> Json {
    mass_source.map_or(Json::Null, |source| Json::text(source.as_str()))
}

fn optional_text(value: Option<&str>) -> Json {
    value.map_or(Json::Null, Json::text)
}

fn run(args: &[OsString]) -> Run<Json> {
    let args: Vec<&str> = args
        .iter()
        .map(|a| {
            a.to_str()
                .filter(|s| s.len() <= 4096)
                .ok_or_else(|| invalid("argument is not bounded UTF-8"))
        })
        .collect::<Run<_>>()?;
    let (command, tail) = args
        .split_first()
        .ok_or_else(|| invalid("missing command"))?;
    if *command != "fastener-geometry" {
        return Err(invalid("unknown command"));
    }
    const OPTIONS: &[&str] = &[
        "--fastener-library",
        "--designation",
        "--length",
        "--count",
        "--washer-under-head",
        "--washer-under-nut",
        "--density",
    ];
    if !tail.len().is_multiple_of(2) || tail.len() > 16 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        if !OPTIONS.contains(key) || options.insert(*key, *value).is_some() {
            return Err(invalid("unknown or repeated option"));
        }
    }
    // The command-line options are admitted before the file is read, so a bad value never
    // depends on a library being present.
    let length = options
        .get("--length")
        .ok_or_else(|| invalid("--length is required"))?
        .parse::<f64>()
        .map_err(|_| invalid("--length is a number of millimetres"))?;
    let count = match options.get("--count") {
        Some(text) => text
            .parse::<u32>()
            .map_err(|_| invalid("--count is a whole number"))?,
        None => 1,
    };
    let density = match options.get("--density") {
        Some(text) => Some(
            text.parse::<f64>()
                .map_err(|_| invalid("--density is a number of kg/m3"))?,
        ),
        None => None,
    };
    // The switch texts are admitted before the file is read; the definition's own preference
    // is the default when a switch is absent.
    let washer_under_head = options
        .get("--washer-under-head")
        .copied()
        .map(boolean)
        .transpose()?;
    let washer_under_nut = options
        .get("--washer-under-nut")
        .copied()
        .map(boolean)
        .transpose()?;
    let path = options
        .get("--fastener-library")
        .ok_or_else(|| invalid("--fastener-library is required"))?;
    let designation = options
        .get("--designation")
        .ok_or_else(|| invalid("--designation is required"))?;
    let library = load_fastener(path)?;
    let lookup = Lookup::new(vec![Source {
        path: (*path).to_owned(),
        library,
    }]);
    let (_source, entry) = lookup.find(designation)?;
    let DefinitionRecord::Fastener { spec } = &entry.record else {
        return Err(invalid("the designation does not name a fastener"));
    };
    let washer_under_head = washer_under_head.unwrap_or(spec.washer_under_head);
    let washer_under_nut = washer_under_nut.unwrap_or(spec.washer_under_nut);
    let build =
        prototype(spec, length, washer_under_head, washer_under_nut).map_err(Refusal::from)?;
    let computed = quantities(spec, &build, count, density).map_err(Refusal::from)?;
    let components = computed
        .components
        .iter()
        .map(|row| component_json(row, &build))
        .collect::<Run<Vec<_>>>()?;
    Ok(Json::object([
        ("outcome", Json::text("fastener_geometry")),
        (
            "digest",
            Json::text(entry.record.resource()?.digest.to_string()),
        ),
        ("designation", optional_text(spec.designation.as_deref())),
        ("length_mm", number(build.length_mm)),
        ("count", Json::uint(u64::from(count))),
        ("density_kg_m3", optional(density)),
        ("components", Json::Array(components)),
        ("volume_each_mm3", number(computed.volume_each_mm3)),
        ("volume_mm3", number(computed.volume_mm3)),
        ("mass_kg", optional(computed.mass_kg)),
        ("mass_source", source_json(computed.mass_source)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(args: &[&str]) -> Run<Json> {
        run(&args.iter().map(OsString::from).collect::<Vec<_>>())
    }

    #[test]
    fn unknown_options_and_bad_values_are_refused_before_any_file_is_read() {
        assert_eq!(
            command(&["fastener-geometry", "--bogus", "x"])
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            command(&["fastener-geometry", "--designation", "M12"])
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            command(&[
                "fastener-geometry",
                "--fastener-library",
                "no-such-file",
                "--designation",
                "M12",
                "--length",
                "50",
                "--washer-under-head",
                "maybe",
            ])
            .unwrap_err()
            .code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            command(&[
                "fastener-geometry",
                "--fastener-library",
                "no-such-file",
                "--designation",
                "M12",
                "--length",
                "50",
                "--count",
                "none",
            ])
            .unwrap_err()
            .code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn the_mass_source_token_follows_the_engine_decision() {
        assert_eq!(
            source_json(Some(MassSource::Declared)),
            Json::text("declared")
        );
        assert_eq!(
            source_json(Some(MassSource::Computed)),
            Json::text("computed")
        );
        assert_eq!(source_json(None), Json::Null);
    }
}
