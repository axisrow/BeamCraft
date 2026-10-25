// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! R01 headless commands: `attach-reference`, `move-reference`, `correct-reference-units`,
//! `write-reference` and `inspect-reference`. They call the engine API through the adapter of
//! `beamcraft_cli::reference` and print one JSON object (the same error shape and exit codes as
//! the other store commands: 0 success, 2 rejection, 3 backend unavailable, 4 outcome unknown).
//!
//! `write-reference` has no success path. It exists so that a write to the source file or to a
//! foreign prim is an observable, scriptable refusal (`FOREIGN_READ_ONLY`, exit 2) and not only a
//! missing feature.

use crate::plate_cmd::{
    Run, footprint_json, id_error, invalid, num, revision_json, translation, triple,
};
use crate::report::report;
use beamcraft_cli::plate::PlateError;
use beamcraft_cli::reference::{
    AttachRequest, Attached, Edited, Ids, ReferenceInspection, Wrapper, attach_reference,
    correct_wrapper_units, inspect_references, move_wrapper, refuse_foreign_write,
};
use beamcraft_host::{
    ids::IdSource,
    revision::{NoFault, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_json::Json;
use beamcraft_model::command::Placement;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId};
use beamcraft_model::reference::{
    CorrectReferenceUnits, ForeignTarget, MoveReference, UnitsInterpretation, UpAxis, WriteForeign,
    check_source_path,
};
use std::collections::BTreeMap;
use std::ffi::OsString;

/// Whether `command` is one of the reference commands.
pub fn handles(command: &std::ffi::OsStr) -> bool {
    command.to_str().is_some_and(|c| {
        matches!(
            c,
            "attach-reference"
                | "move-reference"
                | "correct-reference-units"
                | "write-reference"
                | "inspect-reference"
        )
    })
}

/// Runs one reference command and returns the JSON result and the process exit code.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    report(run(args))
}

const IDS: [&str; 5] = [
    "--op-id",
    "--revision-id",
    "--actor-id",
    "--project-id",
    "--workspace-id",
];

fn options_of(command: &str) -> Option<Vec<&'static str>> {
    let own: &[&str] = match command {
        "attach-reference" => &[
            "--source",
            "--name",
            "--translation",
            "--rotation-z-deg",
            "--new-id",
        ],
        "move-reference" => &["--id", "--translation", "--rotation-z-deg"],
        "correct-reference-units" => &["--id", "--meters-per-unit", "--up-axis", "--clear"],
        "write-reference" => &["--id", "--target", "--prim", "--attribute"],
        "inspect-reference" => &[],
        _ => return None,
    };
    let mut all = vec!["--store"];
    all.extend_from_slice(own);
    if matches!(
        command,
        "attach-reference" | "move-reference" | "correct-reference-units"
    ) {
        all.extend_from_slice(&IDS);
    }
    Some(all)
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
    let allowed = options_of(command).ok_or_else(|| invalid("unknown command"))?;
    if !tail.len().is_multiple_of(2) || tail.len() > 40 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        if !allowed.contains(key) || options.insert(*key, *value).is_some() {
            return Err(invalid("unknown or repeated option"));
        }
    }
    if *command == "attach-reference" {
        // Before the store, the file system or a resolver is touched: a URL or a network path
        // is refused on its text alone.
        let source = options
            .get("--source")
            .ok_or_else(|| invalid("--source is required"))?;
        check_source_path(source).map_err(PlateError::from)?;
    }
    let store_root = options
        .get("--store")
        .ok_or_else(|| invalid("--store is required"))?;
    let blobs = BlobStore::new(store_root).map_err(|e| PlateError {
        code: e.code(),
        message: "store root is not usable",
    })?;
    let store = RevisionStore::new(blobs);
    let _claim = crate::plate_cmd::check_project_option(&store, &options)?;
    match *command {
        "attach-reference" => attach_command(&store, &options),
        "move-reference" => move_command(&store, &options),
        "correct-reference-units" => correct_command(&store, &options),
        "write-reference" => write_command(&store, &options),
        _ => Ok(inspection_json(&inspect_references(&store)?)?),
    }
}

fn ids(options: &BTreeMap<&str, &str>) -> Run<Ids> {
    let mut source = IdSource::system();
    let mut fresh = || {
        source.next_id().map_err(|_| PlateError {
            code: ErrorCode::BackendUnavailable,
            message: "no identity could be generated",
        })
    };
    let given = |key: &str| options.get(key).copied();
    Ok(Ids {
        op_id: match given("--op-id") {
            Some(text) => OpId::parse(text).map_err(id_error)?,
            None => OpId::from_uuid(fresh()?),
        },
        revision_id: match given("--revision-id") {
            Some(text) => RevisionId::parse(text).map_err(id_error)?,
            None => RevisionId::from_uuid(fresh()?),
        },
        actor: match given("--actor-id") {
            Some(text) => Uuid7::parse(text).map_err(PlateError::from)?,
            None => Uuid7::parse(&fresh()?.to_string()).map_err(PlateError::from)?,
        },
        project: match given("--project-id") {
            Some(text) => ProjectId::parse(text).map_err(id_error)?,
            None => ProjectId::from_uuid(fresh()?),
        },
        workspace: match given("--workspace-id") {
            Some(text) => WorkspaceId::parse(text).map_err(id_error)?,
            None => WorkspaceId::from_uuid(fresh()?),
        },
    })
}

fn element_id(options: &BTreeMap<&str, &str>, key: &str) -> Run<ElementId> {
    ElementId::parse(
        options
            .get(key)
            .ok_or_else(|| invalid("an element id is required"))?,
    )
    .map_err(id_error)
}

/// The absolute placement of `--translation` and `--rotation-z-deg` (a plan rotation about Z,
/// in degrees). An omitted option is zero; the placement replaces the earlier one.
fn placement(options: &BTreeMap<&str, &str>) -> Run<Placement> {
    let translation_mm = options
        .get("--translation")
        .map(|t| translation(t))
        .transpose()?
        .unwrap_or([0.0; 3]);
    let degrees = options
        .get("--rotation-z-deg")
        .map(|t| {
            t.parse::<f64>()
                .ok()
                .filter(|d| d.is_finite() && d.abs() <= 3600.0)
                .ok_or_else(|| invalid("the rotation is not a number of degrees within 3600"))
        })
        .transpose()?
        .unwrap_or(0.0);
    let half = degrees.to_radians() / 2.0;
    Ok(Placement {
        translation_mm,
        orientation_xyzw: [0.0, 0.0, half.sin(), half.cos()],
    })
}

fn attach_command(store: &RevisionStore, options: &BTreeMap<&str, &str>) -> Run<Json> {
    let ids = ids(options)?;
    let new_id = match options.get("--new-id") {
        Some(text) => ElementId::parse(text).map_err(id_error)?,
        None => {
            let mut source = IdSource::system();
            ElementId::from_uuid(source.next_id().map_err(|_| PlateError {
                code: ErrorCode::BackendUnavailable,
                message: "no identity could be generated",
            })?)
        }
    };
    let attached = attach_reference(
        store,
        &AttachRequest {
            ids: ids.clone(),
            new_id,
            name: options.get("--name").map(|n| (*n).to_owned()),
            placement: placement(options)?,
            source_path: options
                .get("--source")
                .ok_or_else(|| invalid("--source is required"))?
                .to_string(),
        },
        &mut NoFault,
    )?;
    attached_json(&attached, ids.op_id)
}

fn attached_json(attached: &Attached, op_id: OpId) -> Run<Json> {
    Ok(Json::object([
        (
            "outcome",
            Json::text(if attached.replayed {
                "reference_attach_replayed"
            } else {
                "reference_attached"
            }),
        ),
        ("op_id", Json::text(op_id.to_string())),
        ("reference", wrapper_json(&attached.wrapper)?),
        ("source_reused", Json::Bool(attached.source_reused)),
        ("revision", revision_json(&attached.revision)),
        ("footprint", footprint_json(attached.footprint.as_ref())),
    ]))
}

fn move_command(store: &RevisionStore, options: &BTreeMap<&str, &str>) -> Run<Json> {
    let ids = ids(options)?;
    let edited = move_wrapper(
        store,
        &ids,
        &MoveReference {
            target_id: element_id(options, "--id")?,
            placement: placement(options)?,
        },
        &mut NoFault,
    )?;
    edited_json(&edited, ids.op_id, "reference_moved")
}

fn correct_command(store: &RevisionStore, options: &BTreeMap<&str, &str>) -> Run<Json> {
    let ids = ids(options)?;
    let interpretation = if options.get("--clear") == Some(&"true") {
        if options.contains_key("--meters-per-unit") || options.contains_key("--up-axis") {
            return Err(invalid("--clear cannot be combined with an interpretation"));
        }
        None
    } else if options.contains_key("--clear") {
        return Err(invalid("--clear takes the value true"));
    } else {
        let meters_per_unit = options
            .get("--meters-per-unit")
            .ok_or_else(|| invalid("--meters-per-unit is required (or --clear true)"))?
            .parse::<f64>()
            .map_err(|_| invalid("a number is malformed"))?;
        let up_axis = options
            .get("--up-axis")
            .and_then(|t| UpAxis::parse(t))
            .ok_or_else(|| invalid("--up-axis is required and is Y or Z"))?;
        Some(UnitsInterpretation {
            meters_per_unit,
            up_axis,
        })
    };
    let edited = correct_wrapper_units(
        store,
        &ids,
        &CorrectReferenceUnits {
            target_id: element_id(options, "--id")?,
            interpretation,
        },
        &mut NoFault,
    )?;
    edited_json(&edited, ids.op_id, "reference_units_corrected")
}

fn edited_json(edited: &Edited, op_id: OpId, done: &'static str) -> Run<Json> {
    Ok(Json::object([
        (
            "outcome",
            Json::text(if edited.replayed {
                format!("{done}_replayed")
            } else {
                done.to_owned()
            }),
        ),
        ("op_id", Json::text(op_id.to_string())),
        ("reference", wrapper_json(&edited.wrapper)?),
        (
            "changed_layers",
            Json::Array(edited.changed_layers.iter().map(Json::text).collect()),
        ),
        (
            "foreign_layers",
            Json::Array(
                edited
                    .foreign_layers
                    .iter()
                    .map(|(name, digest)| {
                        Json::object([
                            ("name", Json::text(name.clone())),
                            ("digest", Json::text(digest.to_string())),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("revision", revision_json(&edited.revision)),
        ("footprint", footprint_json(edited.footprint.as_ref())),
    ]))
}

/// `write-reference`: always a rejection. `--target source` addresses the stored source layer;
/// `--prim /DefaultPrim/Path` (optionally with `--attribute`) a prim in the source's namespace.
fn write_command(store: &RevisionStore, options: &BTreeMap<&str, &str>) -> Run<Json> {
    let target = match (
        options.get("--target").copied(),
        options.get("--prim").copied(),
    ) {
        (Some("source"), None) if !options.contains_key("--attribute") => {
            ForeignTarget::SourceLayer
        }
        (None, Some(path)) => ForeignTarget::Prim {
            path: path.to_owned(),
            attribute: options.get("--attribute").map(|a| (*a).to_owned()),
        },
        _ => {
            return Err(invalid(
                "give exactly one of --target source and --prim PATH (--attribute needs --prim)",
            ));
        }
    };
    let command = WriteForeign {
        reference_id: element_id(options, "--id")?,
        target,
    };
    match refuse_foreign_write(store, &command) {
        Ok(never) => match never {},
        Err(error) => Err(error),
    }
}

fn wrapper_json(wrapper: &Wrapper) -> Run<Json> {
    let record = &wrapper.record;
    let [x, y, z, w] = record.placement.orientation_xyzw;
    let source = &record.source;
    let optional = |value: Option<f64>| value.map_or(Ok(Json::Null), num);
    Ok(Json::object([
        ("id", Json::text(record.id.to_string())),
        (
            "path",
            Json::text(
                record
                    .id
                    .prim_path(beamcraft_model::prim::ElementCategory::Reference),
            ),
        ),
        ("name", Json::text(record.name.clone())),
        (
            "placement",
            Json::object([
                ("translation_mm", triple(record.placement.translation_mm)?),
                (
                    "orientation_xyzw",
                    Json::Array(vec![num(x)?, num(y)?, num(z)?, num(w)?]),
                ),
            ]),
        ),
        (
            "source",
            Json::object([
                ("layer", Json::text(wrapper.layer.clone())),
                ("digest", Json::text(source.digest.to_string())),
                ("bytes", Json::uint(source.bytes)),
                ("format", Json::text(source.format.as_str())),
                ("default_prim", Json::text(source.default_prim.clone())),
                ("version", Json::uint(u64::from(source.version))),
                ("label", Json::text(source.label.clone())),
                (
                    "declared_meters_per_unit",
                    optional(source.declared_meters_per_unit)?,
                ),
                (
                    "declared_up_axis",
                    source
                        .declared_up_axis
                        .map_or(Json::Null, |a| Json::text(a.as_str())),
                ),
            ]),
        ),
        (
            "units",
            match &record.units {
                None => Json::Null,
                Some(i) => Json::object([
                    ("meters_per_unit", num(i.meters_per_unit)?),
                    ("up_axis", Json::text(i.up_axis.as_str())),
                    ("scale", num(i.scale())?),
                    ("rotate_x_degrees", optional(i.rotate_x_degrees())?),
                ]),
            },
        ),
        ("units_status", Json::text(wrapper.status.as_str())),
        (
            "foreign_prim_count",
            Json::uint(wrapper.foreign.len() as u64),
        ),
        (
            "foreign_prims",
            Json::Array(
                wrapper
                    .foreign
                    .iter()
                    .map(|f| {
                        Json::object([
                            ("path", Json::text(f.path.clone())),
                            ("source_path", Json::text(f.source_path.clone())),
                            ("type", f.type_name.clone().map_or(Json::Null, Json::text)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]))
}

fn inspection_json(inspection: &ReferenceInspection) -> Run<Json> {
    Ok(Json::object([
        ("outcome", Json::text("reference_inspection")),
        ("revision", revision_json(&inspection.revision)),
        (
            "reference_count",
            Json::uint(inspection.references.len() as u64),
        ),
        (
            "references",
            Json::Array(
                inspection
                    .references
                    .iter()
                    .map(wrapper_json)
                    .collect::<Run<Vec<_>>>()?,
            ),
        ),
        (
            "foreign_layers",
            Json::Array(
                inspection
                    .foreign_layers
                    .iter()
                    .map(|(name, digest, size)| {
                        Json::object([
                            ("name", Json::text(name.clone())),
                            ("digest", Json::text(digest.to_string())),
                            ("bytes", Json::uint(*size)),
                            // Every blob is re-hashed and its size compared when the head opens.
                            ("verified", Json::Bool(true)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]))
}
