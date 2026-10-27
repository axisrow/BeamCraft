// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! T03 CLI option admission; all model mutation goes through the group adapter.
use crate::{
    command_context,
    plate_cmd::{id_error, revision_json},
};
use beamcraft_cli::{
    plate::PlateError,
    transform::{TransformRequest, transform_elements},
};
use beamcraft_engine::{transform::RigidDelta, work_plane::WorkPlane};
use beamcraft_host::{
    ids::IdSource,
    revision::{NoFault, RevisionStore},
    storage::BlobStore,
};
use beamcraft_json::Json;
use beamcraft_model::{error::ErrorCode, id::ElementId};
use std::{collections::BTreeMap, ffi::OsString};
fn error(code: ErrorCode, message: &'static str) -> PlateError {
    PlateError {
        rule_id: None,
        code,
        message,
    }
}
fn invalid(message: &'static str) -> PlateError {
    error(ErrorCode::InvalidArgument, message)
}
pub fn handles(c: &OsString) -> bool {
    c == "move-elements" || c == "rotate-elements" || c == "copy-elements"
}
pub fn execute(args: &[OsString]) -> (Json, u8) {
    crate::report::report(run(args))
}
fn run(args: &[OsString]) -> Result<Json, PlateError> {
    let args = args
        .iter()
        .map(|v| {
            v.to_str()
                .filter(|s| s.len() <= 8192)
                .ok_or_else(|| invalid("argument must be bounded UTF-8"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (command, tail) = args
        .split_first()
        .ok_or_else(|| invalid("missing command"))?;
    if !tail.len().is_multiple_of(2) || tail.len() > 40 {
        return Err(invalid("options must be bounded KEY VALUE pairs"));
    }
    let common = ["--store", "--ids", "--plane"];
    let own: &[&str] = match *command {
        "move-elements" => &["--vector", "--from", "--to"],
        "copy-elements" => &["--vector", "--from", "--to", "--instance"],
        "rotate-elements" => &["--center", "--axis", "--angle"],
        _ => return Err(invalid("unknown transform")),
    };
    let mut options = BTreeMap::new();
    for [k, v] in tail.as_chunks::<2>().0 {
        if !(common.contains(k) || own.contains(k) || command_context::CONTEXT_OPTIONS.contains(k))
            || options.insert(*k, *v).is_some()
        {
            return Err(invalid("unknown or repeated option"));
        }
    }
    let given = |key: &str| options.get(key).copied();
    let need =
        |key: &str| given(key).ok_or_else(|| invalid("required transform option is missing"));
    let mut ids = need("--ids")?
        .split(',')
        .take(129)
        .map(ElementId::parse)
        .collect::<Result<Vec<_>, _>>()
        .map_err(id_error)?;
    ids.sort();
    beamcraft_engine::transform::validate_ids(&ids)?;
    let plane = given("--plane")
        .map(WorkPlane::parse)
        .transpose()?
        .unwrap_or_default();
    let point = |s: &str, reference: [f64; 3]| {
        plane
            .resolve_coordinate(s, reference, beamcraft_engine::coordinate::AxisLock::None)
            .map_err(PlateError::from)
    };
    let world_vector = |v: [f64; 3]| -> Result<[f64; 3], PlateError> {
        let [x, y, z] = plane.to_world(v)?;
        let [a, b, c] = plane.origin();
        Ok([x - a, y - b, z - c])
    };
    let delta = if *command == "rotate-elements" {
        let center = plane.to_world(point(need("--center")?, [0.; 3])?)?;
        let axis = world_vector(point(need("--axis")?, [0.; 3])?)?;
        let angle = need("--angle")?
            .parse::<f64>()
            .map_err(|_| invalid("angle must be radians"))?;
        RigidDelta::rotation(center, axis, angle)?
    } else {
        let vector = match (given("--vector"), given("--from"), given("--to")) {
            (Some(v), None, None) => world_vector(point(v, [0.; 3])?)?,
            (None, Some(a), Some(b)) => {
                let a = point(a, [0.; 3])?;
                let b = point(b, a)?;
                let [x, y, z] = b;
                let [u, v, w] = a;
                world_vector([x - u, y - v, z - w])?
            }
            _ => return Err(invalid("give either --vector or both --from and --to")),
        };
        RigidDelta::translation(vector)?
    };
    let instance = match given("--instance") {
        None | Some("false") => false,
        Some("true") => true,
        _ => return Err(invalid("instance must be true or false")),
    };
    let store = RevisionStore::new(
        BlobStore::new(need("--store")?)
            .map_err(|e| error(e.code(), "store root is not usable"))?,
    );
    let _claim = crate::plate_cmd::check_project_option(&store, &options)?;
    let mut ids_source = IdSource::system();
    let context = command_context::parse(&options, &mut || {
        ids_source.next_id().map_err(|_| {
            error(
                ErrorCode::BackendUnavailable,
                "no identity could be generated",
            )
        })
    })?;
    let result = transform_elements(
        &store,
        &TransformRequest {
            op_id: context.op_id,
            revision_id: context.revision_id,
            actor: context.actor,
            project: context.project,
            workspace: context.workspace,
            ids,
            delta,
            copy: *command == "copy-elements",
            instance,
        },
        &mut NoFault,
    )?;
    Ok(Json::object([
        (
            "outcome",
            Json::text(if result.replayed {
                "transform_replayed"
            } else {
                "transformed"
            }),
        ),
        (
            "elements",
            Json::Array(
                result
                    .elements
                    .iter()
                    .map(|(source, target)| {
                        Json::object([
                            ("source_id", Json::text(source.to_string())),
                            ("element_id", Json::text(target.to_string())),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("revision", revision_json(&result.revision)),
    ]))
}
