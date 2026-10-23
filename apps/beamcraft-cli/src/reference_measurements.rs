// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! W03 immutable saved measurements in a revision journal separate from the referenced model.

use crate::plate::PlateError;
use crate::reference::ForeignMeshResult;
use beamcraft_engine::reference_measure::REFERENCE_MEASUREMENT_ALGORITHM;
use beamcraft_host::ids::IdSource;
use beamcraft_host::revision::{
    ExpectedBase, LayerEntry, LayerRole, NoFault, Revision, RevisionDraft, RevisionStore, Uuid7,
};
use beamcraft_host::storage::BlobStore;
use beamcraft_json::{Json, Limits};
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::error::ErrorCode;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

const INPUT_LAYER: &str = "measurement-inputs.json";
const RESULT_LAYER: &str = "measurement-results.json";
const INPUT_SCHEMA: &str = "beamcraft-measurement-input/1";
const RESULT_SCHEMA: &str = "beamcraft-measurement-result/1";
const STORE_SCHEMA: &[u8] = b"BeamCraft/W03/MeasurementObjects/v1";
type MeasurementLayers = (Vec<Json>, Vec<Json>, Option<Revision>);

fn bad(message: &'static str) -> PlateError {
    PlateError {
        code: ErrorCode::InvalidArgument,
        message,
    }
}

fn integrity(message: &'static str) -> PlateError {
    PlateError {
        code: ErrorCode::IntegrityError,
        message,
    }
}

fn json_error() -> PlateError {
    integrity("saved measurement data is malformed")
}

fn encode(value: &Json) -> Result<Vec<u8>, PlateError> {
    value.encode(&Limits::DEFAULT).map_err(|_| json_error())
}

fn decode(bytes: &[u8]) -> Result<Json, PlateError> {
    let value = beamcraft_json::decode(bytes, &Limits::DEFAULT).map_err(|_| json_error())?;
    if encode(&value)?.as_slice() != bytes {
        return Err(integrity("saved measurement data is not canonical"));
    }
    Ok(value)
}

fn required<'a>(value: &'a Json, key: &str) -> Result<&'a Json, PlateError> {
    value.get(key).map_err(|_| json_error())
}

fn string<'a>(value: &'a Json, key: &str) -> Result<&'a str, PlateError> {
    required(value, key)?.string().map_err(|_| json_error())
}

fn objects<'a>(value: &'a Json, schema: &str) -> Result<&'a [Json], PlateError> {
    value
        .closed(&["schema", "objects"])
        .map_err(|_| json_error())?;
    if string(value, "schema")? != schema {
        return Err(integrity("saved measurement schema is not supported"));
    }
    required(value, "objects")?
        .array()
        .map_err(|_| json_error())
}

fn validate_number_array(value: &Json, len: usize) -> Result<(), PlateError> {
    let values = value.array().map_err(|_| json_error())?;
    if values.len() != len
        || values
            .iter()
            .any(|value| value.float().map_or(true, |number| !number.is_finite()))
    {
        return Err(json_error());
    }
    Ok(())
}

fn decode_number_array<const N: usize>(value: &Json) -> Result<[f64; N], PlateError> {
    let values = value.array().map_err(|_| json_error())?;
    if values.len() != N {
        return Err(json_error());
    }
    let decoded = values
        .iter()
        .map(|value| value.float().map_err(|_| json_error()))
        .collect::<Result<Vec<_>, _>>()?;
    decoded.try_into().map_err(|_| json_error())
}

fn validate_enclosure(value: &Json) -> Result<(), PlateError> {
    if value == &Json::Null {
        return Ok(());
    }
    value
        .closed(&["lower", "upper"])
        .map_err(|_| json_error())?;
    let lower = required(value, "lower")?
        .float()
        .map_err(|_| json_error())?;
    let upper = required(value, "upper")?
        .float()
        .map_err(|_| json_error())?;
    if !lower.is_finite() || !upper.is_finite() || lower > upper {
        return Err(json_error());
    }
    Ok(())
}

fn validate_input(value: &Json) -> Result<(), PlateError> {
    value
        .closed(&[
            "id",
            "reference_id",
            "source_path",
            "source_digest",
            "source_version",
            "model_project_id",
            "model_workspace_id",
            "model_revision_id",
            "model_manifest_digest",
            "units",
            "placement_translation_mm",
            "placement_orientation_xyzw",
            "algorithm",
            "precision",
        ])
        .map_err(|_| json_error())?;
    for key in [
        "id",
        "reference_id",
        "model_project_id",
        "model_workspace_id",
        "model_revision_id",
    ] {
        Uuid7::parse(string(value, key)?).map_err(|_| json_error())?;
    }
    for key in ["source_digest", "model_manifest_digest"] {
        string(value, key)?
            .parse::<ContentDigest>()
            .map_err(|_| json_error())?;
    }
    let source_path = string(value, "source_path")?;
    if source_path.len() < 2
        || !source_path.starts_with('/')
        || required(value, "source_version")?
            .unsigned()
            .map_err(|_| json_error())?
            == 0
        || string(value, "algorithm")? != REFERENCE_MEASUREMENT_ALGORITHM
        || string(value, "precision")? != "outward-rounded-binary64-enclosure"
    {
        return Err(json_error());
    }
    let units = required(value, "units")?;
    if units != &Json::Null {
        units
            .closed(&["meters_per_unit", "up_axis"])
            .map_err(|_| json_error())?;
        let scale = required(units, "meters_per_unit")?
            .float()
            .map_err(|_| json_error())?;
        if !scale.is_finite() || scale <= 0.0 || !matches!(string(units, "up_axis")?, "Y" | "Z") {
            return Err(json_error());
        }
    }
    let translation = required(value, "placement_translation_mm")?;
    let orientation = required(value, "placement_orientation_xyzw")?;
    validate_number_array(translation, 3)?;
    validate_number_array(orientation, 4)?;
    beamcraft_model::command::Placement {
        translation_mm: decode_number_array(translation)?,
        orientation_xyzw: decode_number_array(orientation)?,
    }
    .validate()
    .map_err(|_| json_error())?;
    Ok(())
}

fn validate_result(value: &Json) -> Result<(), PlateError> {
    value
        .closed(&[
            "id",
            "status",
            "input_signature",
            "shell_status",
            "area_mm2",
            "volume_mm3",
            "mass_kg",
            "engineering_quantity_status",
        ])
        .map_err(|_| json_error())?;
    Uuid7::parse(string(value, "id")?).map_err(|_| json_error())?;
    if string(value, "status")?.is_empty()
        || string(value, "shell_status")?.is_empty()
        || string(value, "engineering_quantity_status")? != "unsupported"
        || required(value, "mass_kg")? != &Json::Null
    {
        return Err(json_error());
    }
    let signature = required(value, "input_signature")?;
    if signature != &Json::Null {
        signature
            .string()
            .map_err(|_| json_error())?
            .parse::<ContentDigest>()
            .map_err(|_| json_error())?;
    }
    validate_enclosure(required(value, "area_mm2")?)?;
    validate_enclosure(required(value, "volume_mm3")?)?;
    Ok(())
}

fn document(schema: &str, objects: Vec<Json>) -> Json {
    Json::object([
        ("schema", Json::text(schema)),
        ("objects", Json::Array(objects)),
    ])
}

fn json_number(value: f64) -> Result<Json, PlateError> {
    Json::number(value).map_err(|_| integrity("measurement contains a non-finite number"))
}

fn number_array(values: &[f64]) -> Result<Json, PlateError> {
    Ok(Json::Array(
        values
            .iter()
            .map(|value| json_number(*value))
            .collect::<Result<Vec<_>, _>>()?,
    ))
}

fn enclosure(
    value: Option<beamcraft_engine::reference_measure::MeasurementEnclosure>,
) -> Result<Json, PlateError> {
    value.map_or(Ok(Json::Null), |value| {
        Ok(Json::object([
            ("lower", json_number(value.lo)?),
            ("upper", json_number(value.hi)?),
        ]))
    })
}

fn input_object(id: &str, result: &ForeignMeshResult) -> Result<Json, PlateError> {
    let units = match result.units {
        Some(units) => Json::object([
            ("meters_per_unit", json_number(units.meters_per_unit)?),
            ("up_axis", Json::text(units.up_axis.as_str())),
        ]),
        None => Json::Null,
    };
    Ok(Json::object([
        ("id", Json::text(id)),
        ("reference_id", Json::text(result.reference_id.clone())),
        ("source_path", Json::text(result.source_path.clone())),
        (
            "source_digest",
            Json::text(result.source_digest.to_string()),
        ),
        (
            "source_version",
            Json::uint(u64::from(result.source_version)),
        ),
        (
            "model_project_id",
            Json::text(result.model_project_id.clone()),
        ),
        (
            "model_workspace_id",
            Json::text(result.model_workspace_id.clone()),
        ),
        (
            "model_revision_id",
            Json::text(result.model_revision_id.clone()),
        ),
        (
            "model_manifest_digest",
            Json::text(result.model_manifest_digest.to_string()),
        ),
        ("units", units),
        (
            "placement_translation_mm",
            number_array(&result.placement.translation_mm)?,
        ),
        (
            "placement_orientation_xyzw",
            number_array(&result.placement.orientation_xyzw)?,
        ),
        ("algorithm", Json::text(REFERENCE_MEASUREMENT_ALGORITHM)),
        (
            "precision",
            Json::text("outward-rounded-binary64-enclosure"),
        ),
    ]))
}

fn result_object(id: &str, result: &ForeignMeshResult) -> Result<Json, PlateError> {
    let measurement = result.measurement.as_ref();
    Ok(Json::object([
        ("id", Json::text(id)),
        ("status", Json::text(result.status.clone())),
        (
            "input_signature",
            measurement.map_or(Json::Null, |value| {
                Json::text(value.input_signature.to_string())
            }),
        ),
        (
            "shell_status",
            Json::text(measurement.map_or("unsupported", |value| value.shell_status.as_str())),
        ),
        ("area_mm2", enclosure(measurement.and_then(|m| m.area_mm2))?),
        (
            "volume_mm3",
            enclosure(measurement.and_then(|m| m.volume_mm3))?,
        ),
        ("mass_kg", Json::Null),
        ("engineering_quantity_status", Json::text("unsupported")),
    ]))
}

fn next_id(source: &mut IdSource) -> Result<Uuid7, PlateError> {
    source
        .next_id()
        .map_err(|_| PlateError {
            code: ErrorCode::BackendUnavailable,
            message: "measurement identity could not be generated",
        })
        .and_then(|id| Uuid7::parse(&id.to_string()).map_err(PlateError::from))
}

fn read_layers(revisions: &RevisionStore) -> Result<MeasurementLayers, PlateError> {
    match revisions.open() {
        Ok(revision) => {
            let content = revision.manifest().content();
            let entries = content.entries();
            let [input_entry, result_entry] = entries else {
                return Err(integrity("saved measurement revision has invalid layers"));
            };
            if content.root_layer() != INPUT_LAYER
                || content.schema() != Sha256::digest(STORE_SCHEMA).map_err(|_| json_error())?
                || input_entry.name() != INPUT_LAYER
                || input_entry.role() != LayerRole::Authored
                || result_entry.name() != RESULT_LAYER
                || result_entry.role() != LayerRole::Derived
            {
                return Err(integrity("saved measurement revision failed verification"));
            }
            let inputs = decode(&revision.layer_bytes(INPUT_LAYER)?)?;
            let results = decode(&revision.layer_bytes(RESULT_LAYER)?)?;
            let input_objects = objects(&inputs, INPUT_SCHEMA)?.to_vec();
            let result_objects = objects(&results, RESULT_SCHEMA)?.to_vec();
            let first = input_objects
                .first()
                .ok_or_else(|| integrity("saved measurement revision has no objects"))?;
            if content.project().as_str() != string(first, "model_project_id")?
                || content.workspace().as_str() != string(first, "model_workspace_id")?
                || input_objects.iter().any(|object| {
                    string(object, "model_project_id").ok() != Some(content.project().as_str())
                        || string(object, "model_workspace_id").ok()
                            != Some(content.workspace().as_str())
                })
            {
                return Err(integrity("saved measurement model identity changed"));
            }
            if input_objects.len() != result_objects.len() {
                return Err(integrity(
                    "saved measurement input and result counts disagree",
                ));
            }
            for object in &input_objects {
                validate_input(object)?;
            }
            for object in &result_objects {
                validate_result(object)?;
            }
            let input_ids = input_objects
                .iter()
                .map(|object| string(object, "id").map(str::to_owned))
                .collect::<Result<BTreeSet<_>, _>>()?;
            let result_ids = result_objects
                .iter()
                .map(|object| string(object, "id").map(str::to_owned))
                .collect::<Result<BTreeSet<_>, _>>()?;
            if input_ids.len() != input_objects.len()
                || input_ids != result_ids
                || input_objects
                    .iter()
                    .zip(&result_objects)
                    .any(|(input, result)| string(input, "id").ok() != string(result, "id").ok())
            {
                return Err(integrity("saved measurement object identities disagree"));
            }
            Ok((input_objects, result_objects, Some(revision)))
        }
        Err(beamcraft_host::revision::RevisionError::NoRevision) => {
            Ok((Vec::new(), Vec::new(), None))
        }
        Err(error) => Err(error.into()),
    }
}

fn layer_bytes(schema: &str, values: Vec<Json>) -> Result<Vec<u8>, PlateError> {
    encode(&document(schema, values))
}

fn commit_objects(
    revisions: &RevisionStore,
    project: &str,
    workspace: &str,
    inputs: Vec<Json>,
    results: Vec<Json>,
    previous: Option<Revision>,
) -> Result<(), PlateError> {
    let input_bytes = layer_bytes(INPUT_SCHEMA, inputs)?;
    let result_bytes = layer_bytes(RESULT_SCHEMA, results)?;
    revisions
        .blobs()
        .publish(&input_bytes)
        .map_err(|error| PlateError {
            code: error.code(),
            message: "saved measurement inputs could not be stored",
        })?;
    revisions
        .blobs()
        .publish(&result_bytes)
        .map_err(|error| PlateError {
            code: error.code(),
            message: "saved measurement results could not be stored",
        })?;
    let mut ids = IdSource::system();
    let revision = next_id(&mut ids)?;
    let operation = next_id(&mut ids)?;
    let (expected, generation) = match previous.as_ref() {
        Some(head) => (
            ExpectedBase::Revision(head.info().revision().clone()),
            head.manifest()
                .content()
                .workspace_generation()
                .checked_add(1)
                .ok_or(PlateError {
                    code: ErrorCode::LimitExceeded,
                    message: "measurement revision generation is exhausted",
                })?,
        ),
        None => (ExpectedBase::Initial, 1),
    };
    let schema = Sha256::digest(STORE_SCHEMA).map_err(|_| json_error())?;
    let draft = RevisionDraft {
        project: Uuid7::parse(project).map_err(PlateError::from)?,
        workspace: Uuid7::parse(workspace).map_err(PlateError::from)?,
        revision,
        op: operation,
        workspace_generation: generation,
        schema,
        root_layer: INPUT_LAYER.to_owned(),
        entries: vec![
            LayerEntry::from_bytes(INPUT_LAYER, LayerRole::Authored, &input_bytes)?,
            LayerEntry::from_bytes(RESULT_LAYER, LayerRole::Derived, &result_bytes)?,
        ],
    };
    revisions.commit_with(&draft, &expected, None, &mut NoFault)?;
    Ok(())
}

/// Appends one immutable measurement object for each new input signature.
pub fn save(
    model_store: &RevisionStore,
    results: &[ForeignMeshResult],
) -> Result<Json, PlateError> {
    let first = results
        .first()
        .ok_or_else(|| bad("there are no foreign meshes to save"))?;
    if results.iter().any(|result| {
        result.model_project_id != first.model_project_id
            || result.model_workspace_id != first.model_workspace_id
    }) {
        return Err(integrity(
            "measurement batch spans multiple model workspaces",
        ));
    }
    let measurement_root = model_store.blobs().canonical_root().join("measurements");
    if !measurement_root.exists() {
        match fs::create_dir(&measurement_root) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(bad("measurement store could not be created")),
        }
    }
    let revisions =
        RevisionStore::new(
            BlobStore::new(&measurement_root).map_err(|error| PlateError {
                code: error.code(),
                message: "measurement store is not usable",
            })?,
        );
    let (mut inputs, mut stored_results, previous) = read_layers(&revisions)?;
    let original_count = inputs.len();
    let existing = inputs
        .iter()
        .zip(&stored_results)
        .map(|(input, output)| {
            Ok((
                string(input, "reference_id")?.to_owned(),
                string(input, "source_path")?.to_owned(),
                string(output, "input_signature").ok().map(str::to_owned),
            ))
        })
        .collect::<Result<Vec<_>, PlateError>>()?;
    let mut ids = IdSource::system();
    for result in results {
        let signature = result
            .measurement
            .as_ref()
            .map(|measurement| measurement.input_signature.to_string());
        let exists = signature.as_ref().is_some_and(|signature| {
            existing.iter().any(|(reference, path, prior_signature)| {
                reference == &result.reference_id
                    && path == &result.source_path
                    && prior_signature.as_ref() == Some(signature)
            })
        });
        if exists {
            continue;
        }
        let id = next_id(&mut ids)?.to_string();
        inputs.push(input_object(&id, result)?);
        stored_results.push(result_object(&id, result)?);
    }
    if previous.is_none() || inputs.len() != original_count {
        commit_objects(
            &revisions,
            &first.model_project_id,
            &first.model_workspace_id,
            inputs,
            stored_results,
            previous,
        )?;
    }
    list_at(&revisions, results)
}

/// Lists immutable saved measurement objects and compares each exact input signature to the live model.
pub fn list(
    model_store: &RevisionStore,
    current: &[ForeignMeshResult],
) -> Result<Json, PlateError> {
    let measurement_root = model_store.blobs().canonical_root().join("measurements");
    if !measurement_root.exists() {
        return Ok(Json::object([
            ("outcome", Json::text("saved_measurements")),
            ("objects", Json::Array(Vec::new())),
        ]));
    }
    let revisions =
        RevisionStore::new(
            BlobStore::new(&measurement_root).map_err(|error| PlateError {
                code: error.code(),
                message: "measurement store is not usable",
            })?,
        );
    list_at(&revisions, current)
}

fn list_at(revisions: &RevisionStore, current: &[ForeignMeshResult]) -> Result<Json, PlateError> {
    let (inputs, results, _) = read_layers(revisions)?;
    if let (Some(saved), Some(live)) = (inputs.first(), current.first())
        && (string(saved, "model_project_id")? != live.model_project_id
            || string(saved, "model_workspace_id")? != live.model_workspace_id)
    {
        return Err(bad("saved measurements belong to another model workspace"));
    }
    let live = current
        .iter()
        .map(|result| {
            (
                (result.reference_id.as_str(), result.source_path.as_str()),
                result
                    .measurement
                    .as_ref()
                    .map(|measurement| measurement.input_signature.to_string()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut objects = Vec::with_capacity(inputs.len());
    for (input, output) in inputs.iter().zip(&results) {
        let mut fields = match input {
            Json::Object(fields) => fields.clone(),
            _ => return Err(json_error()),
        };
        let output_fields = match output {
            Json::Object(fields) => fields,
            _ => return Err(json_error()),
        };
        for (key, value) in output_fields {
            if key != "id" {
                fields.insert(key.clone(), value.clone());
            }
        }
        let key = (
            string(input, "reference_id")?,
            string(input, "source_path")?,
        );
        let saved_signature = string(output, "input_signature").ok();
        let freshness = match (saved_signature, live.get(&key).and_then(Option::as_deref)) {
            (Some(saved), Some(now)) if saved == now => "current",
            (None, _) => "unsupported",
            _ => "stale",
        };
        fields.insert("freshness".to_owned(), Json::text(freshness));
        objects.push(Json::Object(fields));
    }
    Ok(Json::object([
        ("outcome", Json::text("saved_measurements")),
        ("objects", Json::Array(objects)),
    ]))
}
