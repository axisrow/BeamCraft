// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D06 headless commands: `annotation-create` and `annotation-list`. They call the engine API
//! through `beamcraft_commands::annotation` and print one JSON object with the same error shape and
//! exit codes as the construction object commands.

use crate::command_context::{self, CONTEXT_OPTIONS};
use crate::plate_cmd::{
    check_project_option, footprint_json, id_error, num, revision_json, triple,
};
use beamcraft_commands::annotation::{
    AnnotationChange, AnnotationContext, annotations, create_annotation, status,
};
use beamcraft_host::{
    ids::IdSource,
    revision::{NoFault, RevisionStore},
    storage::BlobStore,
};
use beamcraft_json::Json;
use beamcraft_model::annotation::{
    AnnotationPayload, CreateAnnotation, FeatureRef, MAX_FEATURE_REF_BYTES,
};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId};
use session::Refusal;
use std::collections::BTreeMap;
use std::ffi::OsString;

type Run<T> = Result<T, Refusal>;

fn invalid(message: &'static str) -> Refusal {
    Refusal {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message,
    }
}

/// Runs one annotation command and returns the JSON result and the process exit code.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    crate::report::report(run(args))
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
        "annotation-create" => &[
            "--store",
            "--name",
            "--kind",
            "--new-id",
            "--from",
            "--to",
            "--anchor",
            "--attribute",
            "--text",
            "--end",
        ],
        "annotation-list" => &["--store"],
        _ => return Err(invalid("unknown command")),
    };
    if !tail.len().is_multiple_of(2) || tail.len() > 40 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        let allowed =
            own.contains(key) || (*command != "annotation-list" && CONTEXT_OPTIONS.contains(key));
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
    if *command == "annotation-list" {
        let found = annotations(&store)?;
        let resolved = status(&store)?;
        return Ok(Json::object([
            ("outcome", Json::text("annotations")),
            ("annotations", list_json(&found, &resolved)?),
        ]));
    }
    let _claim = check_project_option(&store, &options)?;
    let mut source = IdSource::system();
    let mut fresh = || {
        source.next_id().map_err(|_| Refusal {
            rule_id: None,
            code: ErrorCode::BackendUnavailable,
            message: "no identity could be generated",
        })
    };
    let given = |key: &str| options.get(key).copied();
    let common = command_context::parse(&options, &mut fresh)?;
    let context = AnnotationContext {
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
    // One qualified FeatureId text per anchor; the engine admits the full F06 grammar.
    let feature = |key: &'static str| -> Run<FeatureRef> {
        let text = given(key).ok_or_else(|| invalid("an anchor option is required"))?;
        if text.len() > MAX_FEATURE_REF_BYTES || !text.is_ascii() {
            return Err(invalid("a feature id is bounded ASCII of at most 64 bytes"));
        }
        FeatureRef::parse(text).map_err(|_| invalid("the feature anchor is malformed"))
    };
    let payload = match given("--kind").ok_or_else(|| invalid("--kind is required"))? {
        "dimension" => AnnotationPayload::Dimension {
            from: feature("--from")?,
            to: feature("--to")?,
        },
        "mark" => AnnotationPayload::Mark {
            anchor: feature("--anchor")?,
            attribute: given("--attribute")
                .ok_or_else(|| invalid("a mark needs --attribute"))?
                .to_owned(),
        },
        "leader" => AnnotationPayload::Leader {
            anchor: feature("--anchor")?,
            text: given("--text")
                .ok_or_else(|| invalid("a leader needs --text"))?
                .to_owned(),
            end_mm: given("--end")
                .map(coordinates)
                .transpose()?
                .ok_or_else(|| invalid("a leader needs --end X,Y,Z"))?,
        },
        _ => return Err(invalid("--kind is one of dimension, mark or leader")),
    };
    let command = CreateAnnotation {
        annotation: beamcraft_model::annotation::Annotation {
            id: new_id,
            name: given("--name")
                .ok_or_else(|| invalid("--name is required"))?
                .to_owned(),
            // Placeholders of the right length; the store replaces them with the addressed
            // bodies' verified digests at commit time.
            pins: payload
                .anchors()
                .map(|_| ContentDigest::from_bytes([0; 32]))
                .collect(),
            payload,
        },
    };
    let change = create_annotation(&store, &context, &command, &mut NoFault)?;
    change_json(new_id, context.op_id, &change)
}

/// One world coordinate triple from `X,Y,Z`.
fn coordinates(text: &str) -> Run<[f64; 3]> {
    let numbers = text
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|v| !v.is_empty())
        .map(|v| {
            v.parse::<f64>()
                .map_err(|_| invalid("a number is malformed"))
        })
        .collect::<Run<Vec<_>>>()?;
    match numbers.as_slice() {
        [x, y, z] => Ok([*x, *y, *z]),
        _ => Err(invalid("--end takes three coordinates X,Y,Z")),
    }
}

/// The annotations as JSON; `resolved` carries the binding state of each stored record.
pub(crate) fn list_json(
    stored: &[beamcraft_model::annotation::Annotation],
    resolved: &[beamcraft_commands::annotation::AnnotationReport],
) -> Run<Json> {
    let state_of = |id: &ElementId| {
        resolved
            .iter()
            .find(|r| r.annotation.id == *id)
            .map(|r| &r.status.resolution)
    };
    Ok(Json::Array(
        stored
            .iter()
            .map(|a| {
                use beamcraft_engine::annotation::Resolution;
                let mut fields = vec![
                    ("id", Json::text(a.id.to_string())),
                    ("name", Json::text(a.name.clone())),
                    ("kind", Json::text(a.payload.as_str())),
                ];
                match &a.payload {
                    AnnotationPayload::Dimension { .. } => (),
                    AnnotationPayload::Mark { attribute, .. } => {
                        fields.push(("attribute", Json::text(attribute.clone())));
                    }
                    AnnotationPayload::Leader { text, end_mm, .. } => {
                        fields.push(("text", Json::text(text.clone())));
                        fields.push(("end_mm", triple(*end_mm)?));
                    }
                }
                // Every record carries its anchors, empty when nothing resolves: the reply has
                // one shape whatever the binding state.
                let anchors = match state_of(&a.id) {
                    Some(Resolution::Current(bound)) => {
                        fields.push(("status", Json::text("current")));
                        push_bound(&mut fields, bound)?;
                        bound.anchors_mm.clone()
                    }
                    Some(Resolution::Stale(bound)) => {
                        fields.push(("status", Json::text("stale")));
                        push_bound(&mut fields, bound)?;
                        bound.anchors_mm.clone()
                    }
                    Some(Resolution::Unresolved(reason)) => {
                        fields.push(("status", Json::text("unresolved")));
                        fields.push(("reason", Json::text(reason.as_str())));
                        Vec::new()
                    }
                    None => {
                        fields.push(("status", Json::text("unknown")));
                        Vec::new()
                    }
                };
                fields.push((
                    "anchors_mm",
                    Json::Array(
                        anchors
                            .iter()
                            .map(|p| triple(*p))
                            .collect::<Run<Vec<_>>>()?,
                    ),
                ));
                Ok(Json::Object(
                    fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
                ))
            })
            .collect::<Run<Vec<_>>>()?,
    ))
}

fn push_bound(
    fields: &mut Vec<(&'static str, Json)>,
    bound: &beamcraft_engine::annotation::Bound,
) -> Run<()> {
    if let Some(value) = bound.value_mm {
        fields.push(("value_mm", num(value)?));
    }
    if let Some(text) = &bound.text {
        fields.push(("bound_text", Json::text(text.clone())));
    }
    Ok(())
}

fn change_json(subject: ElementId, op_id: OpId, change: &AnnotationChange) -> Run<Json> {
    let outcome = if change.replayed {
        "annotation_created_replayed"
    } else {
        "annotation_created"
    };
    Ok(Json::object([
        ("outcome", Json::text(outcome)),
        ("op_id", Json::text(op_id.to_string())),
        ("element_id", Json::text(subject.to_string())),
        (
            "changed_layers",
            Json::Array(change.changed_layers.iter().map(Json::text).collect()),
        ),
        ("annotations", list_json(&change.annotations, &[])?),
        ("revision", revision_json(&change.revision)),
        ("footprint", footprint_json(change.footprint.as_ref())),
    ]))
}
