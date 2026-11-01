// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! F12 headless commands: `set-project-metadata`, `define-section`, `define-zone`,
//! `show-project`, `connect-project`, `update-connection`, `remove-connection` and
//! `resolve-connection`. They call the adapter through `beamcraft_cli::coordination` and print
//! one JSON object with the same error shape and exit codes as the modelling commands.
//! Shared metadata changes only through these explicit commands (PC-OWN-002); there is no
//! command that resolves `latest` instead of an exact revision (PC-COORD-001).

use crate::command_context::{self, CONTEXT_OPTIONS};
use crate::plate_cmd::{check_project_option, id_error, num, revision_json};
use beamcraft_cli::coordination;
use beamcraft_cli::coordination::{Applied, ConnectionStatus, PinRequest, ResolvedElement};
use beamcraft_host::{
    ids::IdSource,
    revision::{NoFault, RevisionStore},
    storage::BlobStore,
};
use beamcraft_json::Json;
use beamcraft_model::coordination::{SectionRecord, ZoneRecord};
use beamcraft_model::descriptor::{self, ConnectionPlacement, ProjectDescriptor, ScopeSelection};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ConnectionId, ProjectId, RevisionId, SectionId, ZoneId};
use session::Refusal;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

type Run<T> = Result<T, Refusal>;

fn invalid(message: &'static str) -> Refusal {
    Refusal {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message,
    }
}

/// Runs one coordination command and returns the JSON result and the process exit code.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    crate::report::report(run(args))
}

const COMMAND_OPTIONS: &[(&str, &[&str])] = &[
    (
        "set-project-metadata",
        &["--store", "--code", "--name", "--organization"],
    ),
    (
        "define-section",
        &["--store", "--section-id", "--code", "--module", "--label"],
    ),
    (
        "define-zone",
        &["--store", "--zone-id", "--code", "--label"],
    ),
    ("show-project", &["--store"]),
    (
        "connect-project",
        &[
            "--store",
            "--source-store",
            "--source-project",
            "--source-revision",
            "--scope",
            "--connection-id",
            "--label",
            "--translation",
            "--orientation",
        ],
    ),
    (
        "update-connection",
        &[
            "--store",
            "--connection",
            "--expected-revision",
            "--source-store",
            "--source-project",
            "--source-revision",
            "--scope",
            "--label",
            "--translation",
            "--orientation",
        ],
    ),
    ("remove-connection", &["--store", "--connection"]),
    ("resolve-connection", &["--store", "--connection"]),
];

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
    let own = COMMAND_OPTIONS
        .iter()
        .find(|(name, _)| name == command)
        .map(|(_, options)| *options)
        .ok_or_else(|| invalid("unknown command"))?;
    if !tail.len().is_multiple_of(2) || tail.len() > 40 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        let allowed = own.contains(key) || CONTEXT_OPTIONS.contains(key);
        if !allowed || options.insert(*key, *value).is_some() {
            return Err(invalid("unknown or repeated option"));
        }
    }
    let store_root = options
        .get("--store")
        .ok_or_else(|| invalid("--store is required"))?;
    let blobs = BlobStore::new(store_root).map_err(|e| Refusal {
        rule_id: None,
        code: e.code(),
        message: "store root is not usable",
    })?;
    let store = RevisionStore::new(blobs);
    let given = |key: &str| options.get(key).copied();
    let connection_of = |key: &str| -> Run<ConnectionId> {
        let text = given(key).ok_or_else(|| invalid("--connection is required"))?;
        ConnectionId::parse(text).map_err(id_error)
    };
    let mut source = IdSource::system();
    let mut fresh = || {
        source.next_id().map_err(|_| Refusal {
            rule_id: None,
            code: ErrorCode::BackendUnavailable,
            message: "no identity could be generated",
        })
    };
    let _claim = check_project_option(&store, &options)?;
    let common = command_context::parse(&options, &mut fresh)?;
    let hook = &mut NoFault;
    match *command {
        "set-project-metadata" => {
            let code = given("--code");
            let name = given("--name");
            let organization = given("--organization");
            if code.is_none() && name.is_none() && organization.is_none() {
                return Err(invalid(
                    "at least one of --code, --name, --organization is required",
                ));
            }
            let applied = coordination::set_metadata(
                &store,
                &common_context(&common),
                code,
                name,
                organization,
                hook,
            )?;
            updated_json("metadata_updated", &applied)
        }
        "define-section" => {
            let code = given("--code").ok_or_else(|| invalid("--code is required"))?;
            let module = given("--module").ok_or_else(|| invalid("--module is required"))?;
            let modules = module
                .split(',')
                .filter(|token| !token.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let label = given("--label").unwrap_or(code);
            let id = match given("--section-id") {
                Some(text) => SectionId::parse(text).map_err(id_error)?,
                None => SectionId::from_uuid(fresh()?),
            };
            let applied = coordination::define_section(
                &store,
                &common_context(&common),
                SectionRecord {
                    id: id.to_string(),
                    code: code.to_owned(),
                    label: label.to_owned(),
                    modules,
                },
                hook,
            )?;
            updated_json("section_defined", &applied)
        }
        "define-zone" => {
            let code = given("--code").ok_or_else(|| invalid("--code is required"))?;
            let label = given("--label").unwrap_or(code);
            let id = match given("--zone-id") {
                Some(text) => ZoneId::parse(text).map_err(id_error)?,
                None => ZoneId::from_uuid(fresh()?),
            };
            let applied = coordination::define_zone(
                &store,
                &common_context(&common),
                ZoneRecord {
                    id: id.to_string(),
                    code: code.to_owned(),
                    label: label.to_owned(),
                },
                hook,
            )?;
            updated_json("zone_defined", &applied)
        }
        "show-project" => {
            let (descriptor, connections) = coordination::show(&store, &common_context(&common))?;
            project_json(descriptor.as_ref(), &connections)
        }
        "connect-project" => {
            let connection = match given("--connection-id") {
                Some(text) => ConnectionId::parse(text).map_err(id_error)?,
                None => ConnectionId::from_uuid(fresh()?),
            };
            let request = pin_request(&options)?;
            let applied = coordination::connect_project(
                &store,
                &common_context(&common),
                connection,
                &request,
                hook,
            )?;
            updated_json("connection_created", &applied)
        }
        "update-connection" => {
            let connection = connection_of("--connection")?;
            let expected = given("--expected-revision")
                .ok_or_else(|| invalid("--expected-revision is required"))?;
            let expected = RevisionId::parse(expected).map_err(id_error)?;
            let request = pin_request(&options)?;
            let applied = coordination::update_connection(
                &store,
                &common_context(&common),
                connection,
                expected,
                &request,
                hook,
            )?;
            updated_json("connection_updated", &applied)
        }
        "remove-connection" => {
            let connection = connection_of("--connection")?;
            let applied = coordination::remove_connection(
                &store,
                &common_context(&common),
                connection,
                hook,
            )?;
            updated_json("connection_removed", &applied)
        }
        "resolve-connection" => {
            let connection = connection_of("--connection")?;
            let (record, elements) =
                coordination::resolve(&store, &common_context(&common), connection)?;
            resolved_json(&record, &elements)
        }
        _ => Err(invalid("unknown command")),
    }
}

fn common_context(common: &command_context::CommandContext) -> beamcraft_cli::level::LevelContext {
    beamcraft_cli::level::LevelContext {
        op_id: common.op_id,
        revision_id: common.revision_id,
        actor: common.actor.clone(),
        project: common.project,
        workspace: common.workspace,
    }
}

/// Parses the optional placement pair; both values or neither (a partial placement is not an
/// admitted state).
fn placement_of(options: &BTreeMap<&str, &str>) -> Run<Option<ConnectionPlacement>> {
    let translation = options.get("--translation");
    let orientation = options.get("--orientation");
    match (translation, orientation) {
        (None, None) => Ok(None),
        (Some(translation), Some(orientation)) => {
            let translation_mm: [f64; 3] = translation
                .split(',')
                .map(str::parse::<f64>)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| invalid("a placement component is not a number"))?
                .try_into()
                .map_err(|_| invalid("--translation takes x,y,z"))?;
            let orientation_xyzw: [f64; 4] = orientation
                .split(',')
                .map(str::parse::<f64>)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| invalid("a placement component is not a number"))?
                .try_into()
                .map_err(|_| invalid("--orientation takes x,y,z,w"))?;
            Ok(Some(ConnectionPlacement {
                translation_mm,
                orientation_xyzw,
            }))
        }
        _ => Err(invalid(
            "--translation and --orientation are given together",
        )),
    }
}

fn pin_request(options: &BTreeMap<&str, &str>) -> Run<PinRequest> {
    let given = |key: &str| options.get(key).copied();
    let source_store = given("--source-store")
        .map(PathBuf::from)
        .ok_or_else(|| invalid("--source-store is required"))?;
    let source_project =
        given("--source-project").ok_or_else(|| invalid("--source-project is required"))?;
    let source_project = ProjectId::parse(source_project).map_err(id_error)?;
    // Only an exact revision id is a pin: text such as `latest` is refused here (PC-COORD-001).
    let source_revision =
        given("--source-revision").ok_or_else(|| invalid("--source-revision is required"))?;
    let source_revision = RevisionId::parse(source_revision).map_err(id_error)?;
    Ok(PinRequest {
        source_store,
        source_project,
        source_revision,
        scope: given("--scope").map(str::to_owned),
        placement: placement_of(options)?,
        label: given("--label").map(str::to_owned),
    })
}

fn scope_json(scope: &ScopeSelection) -> (Json, Json) {
    match scope {
        ScopeSelection::WholeProject => (Json::text("whole"), Json::Null),
        ScopeSelection::Section(id) => (Json::text("section"), Json::text(id.to_string())),
        ScopeSelection::Zone(id) => (Json::text("zone"), Json::text(id.to_string())),
    }
}

fn descriptor_json(descriptor: &ProjectDescriptor) -> Run<Json> {
    Ok(Json::object([
        ("project", Json::text(descriptor.project.to_string())),
        ("code", Json::text(descriptor.metadata.code.clone())),
        ("name", Json::text(descriptor.metadata.name.clone())),
        (
            "organization",
            match descriptor.metadata.organization.as_deref() {
                Some(organization) => Json::text(organization.to_owned()),
                None => Json::Null,
            },
        ),
        ("frame", Json::text(descriptor.frame.clone())),
        (
            "contract",
            Json::object([
                ("contract", Json::text(descriptor.contract.contract.clone())),
                (
                    "vocabulary",
                    Json::text(descriptor.contract.vocabulary.clone()),
                ),
                ("schema", Json::text(descriptor.contract.schema.clone())),
            ]),
        ),
        (
            "sections",
            Json::Array(
                descriptor
                    .sections
                    .iter()
                    .map(|s| {
                        Json::object([
                            ("id", Json::text(s.id.clone())),
                            ("code", Json::text(s.code.clone())),
                            ("label", Json::text(s.label.clone())),
                            (
                                "modules",
                                Json::Array(s.modules.iter().map(Json::text).collect()),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "zones",
            Json::Array(
                descriptor
                    .zones
                    .iter()
                    .map(|z| {
                        Json::object([
                            ("id", Json::text(z.id.clone())),
                            ("code", Json::text(z.code.clone())),
                            ("label", Json::text(z.label.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "connections",
            Json::Array(
                descriptor
                    .connections
                    .iter()
                    .map(connection_json)
                    .collect::<Run<Vec<_>>>()?,
            ),
        ),
    ]))
}

fn connection_json(record: &descriptor::ConnectionRecord) -> Run<Json> {
    let (scope, scope_id) = scope_json(&record.scope);
    let numbers = |values: &[f64]| -> Run<Json> {
        Ok(Json::Array(
            values.iter().map(|v| num(*v)).collect::<Run<Vec<_>>>()?,
        ))
    };
    let alignment = match &record.placement {
        Some(placement) => Json::object([
            ("translation_mm", numbers(&placement.translation_mm)?),
            ("orientation_xyzw", numbers(&placement.orientation_xyzw)?),
        ]),
        None => Json::text("unknown"),
    };
    Ok(Json::object([
        ("id", Json::text(record.id.to_string())),
        ("label", Json::text(record.label.clone())),
        (
            "source_project",
            Json::text(record.source_project.to_string()),
        ),
        (
            "source_workspace",
            Json::text(record.source_workspace.to_string()),
        ),
        (
            "source_revision",
            Json::text(record.source_revision.to_string()),
        ),
        ("manifest", Json::text(record.manifest.clone())),
        (
            "layers",
            Json::Array(
                record
                    .layers
                    .iter()
                    .map(|(name, digest)| {
                        Json::object([
                            ("layer", Json::text(name.clone())),
                            ("digest", Json::text(digest.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("scope", scope),
        ("scope_id", scope_id),
        ("alignment", alignment),
    ]))
}

fn updated_json(outcome: &'static str, applied: &Applied) -> Run<Json> {
    let outcome = if applied.replayed {
        Json::text("descriptor_replayed")
    } else {
        Json::text(outcome)
    };
    Ok(Json::object([
        ("outcome", outcome),
        ("descriptor", descriptor_json(&applied.descriptor)?),
        (
            "changed_layers",
            Json::Array(applied.changed_layers.iter().map(Json::text).collect()),
        ),
        ("revision", revision_json(&applied.revision)),
    ]))
}

fn project_json(
    descriptor: Option<&ProjectDescriptor>,
    connections: &[ConnectionStatus],
) -> Run<Json> {
    let Some(descriptor) = descriptor else {
        return Ok(Json::object([
            ("outcome", Json::text("project")),
            ("descriptor", Json::Null),
            ("connections", Json::Array(Vec::new())),
        ]));
    };
    let resolved = connections
        .iter()
        .map(|status| match &status.resolution {
            Ok(entries) => Json::object([
                ("id", Json::text(status.record.id.to_string())),
                ("complete", Json::Bool(true)),
                (
                    "pins",
                    Json::Array(
                        entries
                            .iter()
                            .map(|(layer, digest)| {
                                Json::object([
                                    ("layer", Json::text(layer.clone())),
                                    ("digest", Json::text(digest.clone())),
                                ])
                            })
                            .collect(),
                    ),
                ),
            ]),
            Err(error) => Json::object([
                ("id", Json::text(status.record.id.to_string())),
                ("complete", Json::Bool(false)),
                ("code", Json::text(error.code.as_str())),
                ("message", Json::text(error.message)),
            ]),
        })
        .collect();
    Ok(Json::object([
        ("outcome", Json::text("project")),
        ("descriptor", descriptor_json(descriptor)?),
        ("connections", Json::Array(resolved)),
    ]))
}

fn resolved_json(record: &descriptor::ConnectionRecord, elements: &[ResolvedElement]) -> Run<Json> {
    let placement = record
        .placement
        .ok_or_else(|| invalid("the connection has no placement"))?;
    let point = |values: &[f64; 3]| -> Run<Json> {
        Ok(Json::Array(
            values.iter().map(|v| num(*v)).collect::<Run<Vec<_>>>()?,
        ))
    };
    Ok(Json::object([
        ("outcome", Json::text("connection_resolved")),
        ("connection", Json::text(record.id.to_string())),
        ("translation_mm", point(&placement.translation_mm)?),
        (
            "elements",
            Json::Array(
                elements
                    .iter()
                    .map(|element| {
                        Ok(Json::object([
                            ("id", Json::text(element.id.clone())),
                            ("source_mm", point(&element.source_mm)?),
                            ("parent_mm", point(&element.parent_mm)?),
                        ]))
                    })
                    .collect::<Run<Vec<_>>>()?,
            ),
        ),
    ]))
}
