// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A06 headless commands: `create-weld` and `welds`. They call the engine API through
//! `beamcraft_cli::weld` and print one JSON object with the same error shape and exit codes as
//! the plate commands.

use crate::command_context::{self, CONTEXT_OPTIONS};
use crate::plate_cmd::{footprint_json, id_error, revision_json};
use beamcraft_cli::plate::PlateError;
use beamcraft_cli::weld::{WeldChange, WeldContext, WeldView, create_weld, welds};
use beamcraft_host::{
    ids::IdSource,
    revision::{NoFault, RevisionStore},
    storage::BlobStore,
};
use beamcraft_json::Json;
use beamcraft_model::command::{
    CreateWeld, WeldContour, WeldGeometry, WeldLocation, WeldSide, WeldSpec, WeldSymbol, WeldType,
};
use beamcraft_model::error::ErrorCode;
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

/// Whether `command` is one of the weld commands.
pub fn handles(command: &OsString) -> bool {
    command == "create-weld" || command == "welds"
}

/// Runs one weld command and returns the JSON result and the process exit code.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    crate::report::report(run(args))
}

/// Parses `x,y,z;x,y,z;...` world points of a weld path.
fn path(text: &str) -> Run<Vec<[f64; 3]>> {
    let mut points = Vec::new();
    for triple in text.split(';') {
        let mut coordinates = triple.split(',');
        let mut point = [0.0; 3];
        for slot in &mut point {
            *slot = coordinates
                .next()
                .and_then(|c| c.parse::<f64>().ok())
                .ok_or_else(|| invalid("--path is X,Y,Z;X,Y,Z;... world points"))?;
        }
        if coordinates.next().is_some() {
            return Err(invalid("--path is X,Y,Z;X,Y,Z;... world points"));
        }
        points.push(point);
    }
    Ok(points)
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
        "create-weld" => &[
            "--store",
            "--name",
            "--new-id",
            "--main",
            "--secondary",
            "--type",
            "--size",
            "--location",
            "--side",
            "--path",
            "--all-around",
            "--contour",
        ],
        "welds" => &["--store"],
        _ => return Err(invalid("unknown command")),
    };
    if !tail.len().is_multiple_of(2) || tail.len() > 40 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        let allowed = own.contains(key) || (*command != "welds" && CONTEXT_OPTIONS.contains(key));
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
    if *command == "welds" {
        return Ok(Json::object([
            ("outcome", Json::text("welds")),
            ("welds", views_json(&welds(&store)?)),
        ]));
    }
    let given = |key: &str| options.get(key).copied();
    let required = |key: &str, missing: &'static str| given(key).ok_or_else(|| invalid(missing));
    // Every weld property is authored explicitly: no option has a default except the type, of
    // which `fillet` is the only supported value.
    let weld = WeldSpec {
        weld_type: match given("--type") {
            Some(t) => WeldType::parse(t).ok_or(PlateError {
                rule_id: None,
                code: ErrorCode::UnsupportedGeometry,
                message: "--type supports fillet only",
            })?,
            None => WeldType::Fillet,
        },
        size_mm: required("--size", "--size is required")?
            .parse::<f64>()
            .map_err(|_| invalid("--size is a number of millimetres"))?,
        location: WeldLocation::parse(required("--location", "--location is required")?)
            .ok_or_else(|| invalid("--location is workshop or site"))?,
        side: WeldSide::parse(required("--side", "--side is required")?)
            .ok_or_else(|| invalid("--side is arrow, other or both"))?,
    };
    // The optional A15 geometry: a path switches the solid and its symbol data on; the symbol
    // options alone have no supported meaning.
    let geometry = match given("--path") {
        Some(text) => Some(WeldGeometry {
            path_mm: path(text)?,
            symbol: WeldSymbol {
                all_around: match given("--all-around") {
                    None => false,
                    Some("false") => false,
                    Some("true") => true,
                    Some(_) => return Err(invalid("--all-around is true or false")),
                },
                contour: match given("--contour") {
                    None => WeldContour::None,
                    Some(token) => WeldContour::parse(token)
                        .ok_or_else(|| invalid("--contour is none, flush, concave or convex"))?,
                },
            },
        }),
        None => {
            if options.contains_key("--all-around") || options.contains_key("--contour") {
                return Err(invalid("--all-around and --contour need --path"));
            }
            None
        }
    };
    let element = |key: &str, missing: &'static str| -> Run<ElementId> {
        ElementId::parse(required(key, missing)?).map_err(id_error)
    };
    let (main_id, secondary_id) = (
        element("--main", "--main is required")?,
        element("--secondary", "--secondary is required")?,
    );
    let name = required("--name", "--name is required")?.to_owned();
    let mut source = IdSource::system();
    let mut fresh = || {
        source.next_id().map_err(|_| PlateError {
            rule_id: None,
            code: ErrorCode::BackendUnavailable,
            message: "no identity could be generated",
        })
    };
    let common = command_context::parse(&options, &mut fresh)?;
    let context = WeldContext {
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
    let command = CreateWeld {
        new_id,
        name,
        main_id,
        secondary_id,
        weld,
        geometry,
    };
    let change = create_weld(&store, &context, &command, &mut NoFault)?;
    Ok(change_json(new_id, context.op_id, &change))
}

fn views_json(views: &[WeldView]) -> Json {
    Json::Array(
        views
            .iter()
            .map(|v| {
                let mut fields = vec![
                    ("id", Json::text(v.id.to_string())),
                    ("name", Json::text(v.name.clone())),
                    ("main", Json::text(v.main.to_string())),
                    ("secondary", Json::text(v.secondary.to_string())),
                    ("type", Json::text(v.spec.weld_type.as_token())),
                    ("size_mm", Json::text(v.spec.size_mm.to_string())),
                    ("location", Json::text(v.spec.location.as_token())),
                    ("side", Json::text(v.spec.side.as_token())),
                ];
                match &v.solid {
                    None => fields.push(("path_mm", Json::Null)),
                    Some(solid) => {
                        fields.push((
                            "path_mm",
                            Json::Array(
                                solid
                                    .geometry
                                    .path_mm
                                    .iter()
                                    .map(|p| {
                                        Json::Array(
                                            p.iter().map(|c| Json::text(c.to_string())).collect(),
                                        )
                                    })
                                    .collect(),
                            ),
                        ));
                        fields.push(("all_around", Json::Bool(solid.geometry.symbol.all_around)));
                        fields.push((
                            "contour",
                            Json::text(solid.geometry.symbol.contour.as_token()),
                        ));
                        fields.push(("length_mm", Json::text(solid.length_mm.to_string())));
                        fields.push(("volume_mm3", Json::text(solid.volume_mm3.to_string())));
                    }
                }
                Json::Object(
                    fields
                        .into_iter()
                        .map(|(k, v)| (k.to_string(), v))
                        .collect(),
                )
            })
            .collect(),
    )
}

fn change_json(subject: ElementId, op_id: OpId, change: &WeldChange) -> Json {
    let outcome = if change.replayed {
        "weld_created_replayed"
    } else {
        "weld_created"
    };
    Json::object([
        ("outcome", Json::text(outcome)),
        ("op_id", Json::text(op_id.to_string())),
        ("element_id", Json::text(subject.to_string())),
        (
            "changed_layers",
            Json::Array(change.changed_layers.iter().map(Json::text).collect()),
        ),
        ("welds", views_json(&change.welds)),
        ("revision", revision_json(&change.revision)),
        ("footprint", footprint_json(change.footprint.as_ref())),
    ])
}
