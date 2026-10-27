// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A05 and A14.3 headless commands: `create-bolt` (the bolt amendment's form), `bolts` and
//! `bolt-groups`. They call the engine API through `beamcraft_cli::bolt` and `bolt_group` and
//! print one JSON object with the same error shape and exit codes as the weld commands.

use crate::command_context::{self, CONTEXT_OPTIONS};
use crate::plate_cmd::{footprint_json, id_error, revision_json};
use beamcraft_cli::bolt::{BoltView, bolts};
use beamcraft_cli::bolt_group::{
    BoltGroupContext, CreatedGroup, FastenerGroupView, create_bolt_group, groups,
};
use beamcraft_cli::definition::{NameResolution, resolve_name};
use beamcraft_cli::library;
use beamcraft_cli::plate::PlateError;
use beamcraft_host::{
    ids::IdSource,
    revision::{NoFault, RevisionStore},
    storage::BlobStore,
};
use beamcraft_json::Json;
use beamcraft_model::command::{BoltGroupPlacement, BoltLengthMode, CreateBoltGroup};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::fastener::HoleMode;
use beamcraft_model::id::{ElementId, OpId};
use std::collections::BTreeMap;
use std::ffi::OsString;

type Run<T> = Result<T, PlateError>;

fn invalid(message: &'static str) -> PlateError {
    PlateError {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message,
    }
}

/// Whether `command` is one of the bolt commands.
pub fn handles(command: &OsString) -> bool {
    command == "create-bolt" || command == "bolts" || command == "bolt-groups"
}

/// Runs one bolt command and returns the JSON result and the process exit code.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    crate::report::report(run(args))
}

/// Parses `X,Y,Z` as three finite numbers of millimetres.
fn vec3(text: &str) -> Run<[f64; 3]> {
    let parts: Option<Vec<f64>> = text
        .split(',')
        .map(|p| p.trim().parse::<f64>().ok())
        .collect();
    let Some(values) = parts else {
        return Err(invalid("the vector is three comma-separated numbers"));
    };
    let v: [f64; 3] = <[f64; 3]>::try_from(values.as_slice())
        .map_err(|_| invalid("the vector is three comma-separated numbers"))?;
    if v.iter().any(|n| !n.is_finite()) {
        return Err(PlateError {
            rule_id: None,
            code: ErrorCode::NonFiniteNumber,
            message: "the vector is three comma-separated numbers",
        });
    }
    Ok(v)
}

/// Parses a comma-separated list of positive finite numbers (the spacings).
fn spacings(text: &str) -> Run<Vec<f64>> {
    let values: Option<Vec<f64>> = text
        .split(',')
        .map(|p| p.trim().parse::<f64>().ok())
        .collect();
    let Some(values) = values else {
        return Err(invalid("the spacings are comma-separated numbers"));
    };
    if values.iter().any(|v| !v.is_finite()) {
        return Err(PlateError {
            rule_id: None,
            code: ErrorCode::NonFiniteNumber,
            message: "the spacings are comma-separated numbers",
        });
    }
    Ok(values)
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
    let own: &[&str] = match *command {
        "create-bolt" => &[
            "--store",
            "--name",
            "--new-id",
            "--parts",
            "--fastener",
            "--fastener-library",
            "--point1",
            "--point2",
            "--plane-normal",
            "--rotation",
            "--on-plane",
            "--depth",
            "--offset-start",
            "--spacings",
            "--count",
            "--pitch",
            "--offset-end",
            "--length-mode",
            "--fixed-length",
            "--washer-under-head",
            "--washer-under-nut",
            "--hole-mode",
            "--hole-clearance",
        ],
        "bolts" | "bolt-groups" => &["--store"],
        _ => return Err(invalid("unknown command")),
    };
    if !tail.len().is_multiple_of(2) || tail.len() > 48 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        let allowed = own.contains(key) || (*command != "bolts" && CONTEXT_OPTIONS.contains(key));
        if !allowed || options.insert(*key, *value).is_some() {
            return Err(invalid("unknown or repeated option"));
        }
    }
    let store_root = options
        .get("--store")
        .ok_or_else(|| invalid("--store is required"))?;
    let blobs = BlobStore::new(store_root).map_err(|e| PlateError {
        rule_id: None,
        code: e.code(),
        message: "store root is not usable",
    })?;
    let store = RevisionStore::new(blobs);
    let _claim = crate::plate_cmd::check_project_option(&store, &options)?;
    if *command == "bolts" {
        return Ok(Json::object([
            ("outcome", Json::text("bolts")),
            ("bolts", views_json(&bolts(&store)?)),
        ]));
    }
    if *command == "bolt-groups" {
        return Ok(Json::object([
            ("outcome", Json::text("bolt_groups")),
            ("groups", groups_json(&groups(&store)?)),
        ]));
    }
    let given = |key: &str| options.get(key).copied();
    let required = |key: &str, missing: &'static str| given(key).ok_or_else(|| invalid(missing));
    let number_or = |key: &'static str| -> Run<f64> {
        given(key)
            .map(|text| text.parse::<f64>())
            .transpose()
            .map_err(|_| invalid("the value is a number"))?
            .map_or(Ok(0.0), Ok)
    };
    // The group line and the reference plane.
    let point1 = vec3(required("--point1", "--point1 is required")?)?;
    let point2 = vec3(required("--point2", "--point2 is required")?)?;
    let plane_normal = vec3(required("--plane-normal", "--plane-normal is required")?)?;
    let rotation_deg = number_or("--rotation")?;
    let placement = BoltGroupPlacement {
        point1_mm: point1,
        point2_mm: point2,
        plane_normal,
        rotation_rad: rotation_deg * core::f64::consts::PI / 180.0,
        on_plane_mm: number_or("--on-plane")?,
        depth_mm: number_or("--depth")?,
    };
    let offset_start_mm = number_or("--offset-start")?;
    // The row: explicit spacings, or a generator expanded here; the spacings stay authoritative.
    let spacings_mm = if let Some(text) = given("--spacings") {
        spacings(text)?
    } else {
        let count: usize = required("--count", "--count or --spacings is required")?
            .parse::<usize>()
            .map_err(|_| invalid("--count is a whole number of fasteners"))?;
        if count == 0 {
            return Err(invalid("--count is at least one fastener"));
        }
        match (given("--pitch"), given("--offset-end")) {
            (Some(pitch), None) => {
                let pitch: f64 = pitch.parse().map_err(|_| invalid("--pitch is a number"))?;
                vec![pitch; count.saturating_sub(1)]
            }
            (None, Some(end)) => {
                let end: f64 = end
                    .parse()
                    .map_err(|_| invalid("--offset-end is a number"))?;
                let span = end - offset_start_mm;
                if count == 1 || span <= 0.0 {
                    Vec::new()
                } else {
                    let step = span / (count - 1) as f64;
                    (1..count).map(|i| step * i as f64).collect()
                }
            }
            (Some(_), Some(_)) => {
                return Err(invalid("--pitch and --offset-end are alternatives"));
            }
            (None, None) => {
                return Err(invalid("--pitch or --offset-end is required with --count"));
            }
        }
    };
    let length_mode = match given("--length-mode") {
        None => BoltLengthMode::Auto,
        Some("auto") => BoltLengthMode::Auto,
        Some("fixed") => BoltLengthMode::Fixed,
        Some(_) => return Err(invalid("--length-mode is auto or fixed")),
    };
    let fixed_length_mm = match length_mode {
        BoltLengthMode::Fixed => Some(
            required(
                "--fixed-length",
                "--fixed-length is required for a fixed length",
            )?
            .parse::<f64>()
            .map_err(|_| invalid("--fixed-length is a number"))?,
        ),
        BoltLengthMode::Auto => None,
    };
    let switch = |key: &'static str| -> Run<Option<bool>> {
        match given(key) {
            None => Ok(None),
            Some("true") => Ok(Some(true)),
            Some("false") => Ok(Some(false)),
            Some(_) => Err(invalid("a washer switch is true or false")),
        }
    };
    let hole_mode = match given("--hole-mode") {
        None => HoleMode::Generate,
        Some("generate") => HoleMode::Generate,
        Some("none") => HoleMode::None,
        Some(_) => return Err(invalid("--hole-mode is generate or none")),
    };
    let hole_clearance_mm = match given("--hole-clearance") {
        None => None,
        Some(text) => Some(
            text.parse::<f64>()
                .map_err(|_| invalid("--hole-clearance is a number"))?,
        ),
    };
    let parts = required("--parts", "--parts is required")?
        .split(',')
        .map(|p| ElementId::parse(p.trim()).map_err(id_error))
        .collect::<Run<Vec<_>>>()?;
    let name = required("--name", "--name is required")?.to_owned();
    // The fastener: a library entry resolves and pins in this transaction; a designation
    // without a library resolves among the head's pinned definitions.
    let designation = required("--fastener", "--fastener is required")?;
    let (record, fastener, pin) = match given("--fastener-library") {
        Some(path) => {
            let loaded = library::load_fastener(path)?;
            let found = loaded.entry(designation)?;
            let record = found.record.clone();
            let fastener = record.resource()?.digest;
            (record, fastener, true)
        }
        None => {
            let definitions = beamcraft_cli::definition::head_definitions(&store)?;
            let digest = match resolve_name(&definitions, designation, true) {
                NameResolution::Found(digest) => digest,
                NameResolution::NotFound => {
                    return Err(invalid(
                        "the designation is not pinned in the head; name a --fastener-library",
                    ));
                }
                NameResolution::Ambiguous(_) => {
                    return Err(invalid("the designation is ambiguous in the head"));
                }
            };
            let missing = || PlateError {
                rule_id: None,
                code: ErrorCode::IntegrityError,
                message: "a resolved definition is absent from the head",
            };
            let record = definitions.get(&digest).cloned().ok_or_else(missing)?;
            (record, digest, false)
        }
    };
    let mut source = IdSource::system();
    let mut fresh = || {
        source.next_id().map_err(|_| PlateError {
            rule_id: None,
            code: ErrorCode::BackendUnavailable,
            message: "no identity could be generated",
        })
    };
    let common = command_context::parse(&options, &mut fresh)?;
    let context = BoltGroupContext {
        op_id: common.op_id,
        revision_id: common.revision_id,
        actor: common.actor,
        project: common.project,
        workspace: common.workspace,
    };
    let new_id = match given("--new-id") {
        Some(text) => ElementId::parse(text).map_err(id_error)?,
        None => ElementId::from_uuid(fresh()?),
    };
    let command = CreateBoltGroup {
        new_id,
        name,
        parts,
        fastener,
        placement,
        offset_start_mm,
        spacings_mm,
        length_mode,
        fixed_length_mm,
        washer_under_head: switch("--washer-under-head")?,
        washer_under_nut: switch("--washer-under-nut")?,
        hole_mode,
        hole_clearance_mm,
    };
    let change = create_bolt_group(&store, &context, &command, &record, pin, &mut NoFault)?;
    Ok(groups_json_change(new_id, context.op_id, &change))
}

/// The A05 bolt listing, unchanged.
fn views_json(views: &[BoltView]) -> Json {
    Json::Array(
        views
            .iter()
            .map(|v| {
                Json::object([
                    ("id", Json::text(v.id.to_string())),
                    ("name", Json::text(v.name.clone())),
                    ("main", Json::text(v.main.to_string())),
                    ("secondary", Json::text(v.secondary.to_string())),
                    ("diameter_mm", Json::text(v.spec.diameter_mm.to_string())),
                    ("count", Json::text(v.spec.count.to_string())),
                    ("pitch_mm", Json::text(v.spec.pitch_mm.to_string())),
                    (
                        "origin_mm",
                        Json::Array(
                            v.spec
                                .origin_mm
                                .iter()
                                .map(|n| Json::text(n.to_string()))
                                .collect(),
                        ),
                    ),
                    (
                        "direction_mm",
                        Json::Array(
                            v.spec
                                .direction_mm
                                .iter()
                                .map(|n| Json::text(n.to_string()))
                                .collect(),
                        ),
                    ),
                ])
            })
            .collect(),
    )
}

fn groups_json(groups: &[FastenerGroupView]) -> Json {
    Json::Array(groups.iter().map(group_json).collect())
}

fn group_json(group: &FastenerGroupView) -> Json {
    let number = |value: f64| Json::number(value).unwrap_or(Json::Null);
    Json::object([
        ("id", Json::text(group.id.to_string())),
        ("name", Json::text(group.name.clone())),
        (
            "parts",
            Json::Array(
                group
                    .connects
                    .iter()
                    .map(|p| Json::text(p.to_string()))
                    .collect(),
            ),
        ),
        ("fastener_digest", Json::text(group.fastener.to_string())),
        ("fastener_count", number(group.command.count() as f64)),
        ("length_mm", number(group.length_mm)),
        ("length_mode", Json::text(group.command.length_mode.token())),
        ("hole_mode", Json::text(group.command.hole_mode.token())),
    ])
}

fn groups_json_change(subject: ElementId, op_id: OpId, change: &CreatedGroup) -> Json {
    let outcome = if change.replayed {
        "bolt_group_created_replayed"
    } else {
        "bolt_group_created"
    };
    Json::object([
        ("outcome", Json::text(outcome)),
        ("op_id", Json::text(op_id.to_string())),
        ("element_id", Json::text(subject.to_string())),
        (
            "changed_layers",
            Json::Array(change.changed_layers.iter().map(Json::text).collect()),
        ),
        ("groups", groups_json(&change.groups)),
        ("revision", revision_json(&change.revision)),
        ("footprint", footprint_json(change.footprint.as_ref())),
    ])
}
