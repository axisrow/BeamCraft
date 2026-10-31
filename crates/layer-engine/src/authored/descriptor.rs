// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The F12 project descriptor record of the root layer (PC sections 2-5).
//!
//! The descriptor and its coordination connections live in the root layer's
//! `customLayerData` under the reserved key `bimProjectDescriptor`, as nested dictionaries
//! whose values are text only: identities as full UUIDv7 or 32-hex-digit forms, digests as
//! 64 lowercase hex digits, a placement as its translation and unit-quaternion components in
//! canonical decimal form. The reserved key is written only through this module's
//! `write_root_descriptor` and decoded only through `descriptor_of`, which revalidates every
//! rule (`model::descriptor`), so a hostile or hand-edited record is a typed refusal, never
//! a silently ignored or repaired value.
//!
//! The frozen U02 schema descriptor is unchanged: old stores keep their pin, and this record
//! is optional metadata additional to it.

use super::edit::{field, path, serialize};
use crate::{Diagnostic, Value, corrupt, fail};
use beamcraft_model::command::CommandError;
use beamcraft_model::coordination::{ProjectMetadata, SectionRecord, ZoneRecord};
use beamcraft_model::descriptor::{
    ConnectionPlacement, ConnectionRecord, ProjectDescriptor, ScopeSelection,
};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{
    ConnectionId, ProjectId, RevisionId, SectionId, Uuid7, WorkspaceId, ZoneId,
};
use openusd::sdf::{self, AbstractData};
use std::collections::BTreeMap;

/// Key of the F12 descriptor record in the root layer's `customLayerData`.
pub const DESCRIPTOR_KEY: &str = "bimProjectDescriptor";

/// Maps a model rule refusal onto the layer diagnostic, keeping code, message and F12 rule.
fn model_fail(error: CommandError) -> Diagnostic {
    Diagnostic {
        code: error.code,
        message: error.message,
        rule_id: error.rule_id,
    }
}

fn bad_record() -> Diagnostic {
    fail(
        ErrorCode::IntegrityError,
        "invalid project descriptor record",
    )
}

fn text_of(value: &Value) -> Result<&str, Diagnostic> {
    match value {
        Value::Text(text) => Ok(text),
        _ => Err(bad_record()),
    }
}

fn dict_of(value: &Value) -> Result<&BTreeMap<String, Value>, Diagnostic> {
    match value {
        Value::Dictionary(dict) => Ok(dict),
        _ => Err(bad_record()),
    }
}

fn required_text(dict: &BTreeMap<String, Value>, key: &str) -> Result<String, Diagnostic> {
    text_of(dict.get(key).ok_or_else(bad_record)?).map(str::to_owned)
}

/// Parses the 32-hex-digit identity form of a registry key.
fn uuid_from_hex32(text: &str) -> Result<Uuid7, Diagnostic> {
    let bytes = text.as_bytes();
    if bytes.len() != 32
        || !bytes
            .iter()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(fail(ErrorCode::InvalidId, "invalid registry identity"));
    }
    let mut out = [0u8; 16];
    let pairs = bytes.as_chunks::<2>().0.iter().zip(out.iter_mut());
    for (pair, slot) in pairs {
        let hi = (pair[0] as char).to_digit(16).ok_or_else(bad_record)?;
        let lo = (pair[1] as char).to_digit(16).ok_or_else(bad_record)?;
        *slot = ((hi << 4) | lo) as u8;
    }
    Uuid7::from_bytes(out).map_err(|_| fail(ErrorCode::InvalidId, "invalid registry identity"))
}

/// Parses the full canonical UUIDv7 text form.
fn parse_uuid7(text: &str) -> Result<Uuid7, Diagnostic> {
    Uuid7::parse(text).map_err(|_| fail(ErrorCode::InvalidId, "invalid registry identity"))
}

/// Encodes a placement as its two bounded decimal texts (translation, then quaternion `xyzw`).
fn encode_placement(placement: &ConnectionPlacement) -> (String, String) {
    let t = &placement.translation_mm;
    let q = &placement.orientation_xyzw;
    (
        format!("{},{},{}", t[0], t[1], t[2]),
        format!("{},{},{},{}", q[0], q[1], q[2], q[3]),
    )
}

fn decode_f64_list(text: &str, expected: usize) -> Result<Vec<f64>, Diagnostic> {
    let values = text
        .split(',')
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| bad_record())?;
    if values.len() != expected {
        return Err(bad_record());
    }
    Ok(values)
}

/// Decodes one connection record dictionary.
fn decode_connection(dict: &BTreeMap<String, Value>) -> Result<ConnectionRecord, Diagnostic> {
    let scope = match required_text(dict, "scope")?.as_str() {
        "whole" => ScopeSelection::WholeProject,
        "section" => ScopeSelection::Section(SectionId::from_uuid(parse_uuid7(&required_text(
            dict, "scopeId",
        )?)?)),
        "zone" => ScopeSelection::Zone(ZoneId::from_uuid(parse_uuid7(&required_text(
            dict, "scopeId",
        )?)?)),
        _ => return Err(bad_record()),
    };
    let placement = if required_text(dict, "alignment")? == "unknown" {
        None
    } else {
        let translation: [f64; 3] = decode_f64_list(&required_text(dict, "translationMm")?, 3)?
            .try_into()
            .map_err(|_| bad_record())?;
        let orientation_xyzw: [f64; 4] =
            decode_f64_list(&required_text(dict, "orientationXyzw")?, 4)?
                .try_into()
                .map_err(|_| bad_record())?;
        let placement = ConnectionPlacement {
            translation_mm: translation,
            orientation_xyzw,
        };
        placement.validate().map_err(model_fail)?;
        Some(placement)
    };
    let layers = beamcraft_model::descriptor::decode_layers(&required_text(dict, "layers")?)
        .map_err(model_fail)?;
    Ok(ConnectionRecord {
        id: ConnectionId::from_uuid(parse_uuid7(&required_text(dict, "id")?)?),
        source_project: ProjectId::from_uuid(parse_uuid7(&required_text(dict, "sourceProject")?)?),
        source_workspace: WorkspaceId::from_uuid(parse_uuid7(&required_text(
            dict,
            "sourceWorkspace",
        )?)?),
        source_revision: RevisionId::from_uuid(parse_uuid7(&required_text(
            dict,
            "sourceRevision",
        )?)?),
        manifest: required_text(dict, "manifest")?,
        layers,
        scope,
        placement,
        label: required_text(dict, "label")?,
    })
}

/// Decodes the descriptor record of composed layer metadata, validating every rule. `None` for
/// a layer that carries no descriptor; any malformed record is a typed refusal, never `None`.
pub fn descriptor_of(
    metadata: &BTreeMap<String, Value>,
) -> Result<Option<ProjectDescriptor>, Diagnostic> {
    let Some(Value::Dictionary(data)) = metadata.get("customLayerData") else {
        return Ok(None);
    };
    let Some(record) = data.get(DESCRIPTOR_KEY) else {
        return Ok(None);
    };
    let dict = dict_of(record)?;
    let project = ProjectId::from_uuid(uuid_from_hex32(&required_text(dict, "project")?)?);
    let organization = required_text(dict, "organization")?;
    let descriptor = ProjectDescriptor {
        project,
        metadata: ProjectMetadata {
            code: required_text(dict, "code")?,
            name: required_text(dict, "name")?,
            organization: (!organization.is_empty()).then_some(organization),
        },
        contract: beamcraft_model::descriptor::ContractRequirements {
            contract: required_text(dict, "contract")?,
            vocabulary: required_text(dict, "vocabulary")?,
            schema: required_text(dict, "schema")?,
        },
        frame: required_text(dict, "frame")?,
        sections: decode_registry(dict.get("sections"), decode_section)?,
        zones: decode_registry(dict.get("zones"), decode_zone)?,
        connections: decode_registry(dict.get("connections"), decode_connection)?,
    };
    descriptor.validate().map_err(model_fail)?;
    Ok(Some(descriptor))
}

fn decode_section(dict: &BTreeMap<String, Value>) -> Result<SectionRecord, Diagnostic> {
    Ok(SectionRecord {
        id: required_text(dict, "id")?,
        code: required_text(dict, "code")?,
        label: required_text(dict, "label")?,
        modules: required_text(dict, "modules")?
            .split(',')
            .filter(|token| !token.is_empty())
            .map(str::to_owned)
            .collect(),
    })
}

fn decode_zone(dict: &BTreeMap<String, Value>) -> Result<ZoneRecord, Diagnostic> {
    Ok(ZoneRecord {
        id: required_text(dict, "id")?,
        code: required_text(dict, "code")?,
        label: required_text(dict, "label")?,
    })
}

/// Decodes one registry dictionary keyed by identity; every key must agree with the record's
/// own `id` entry, so a registry cannot hold two spellings of one identity.
fn decode_registry<T, F>(value: Option<&Value>, decode: F) -> Result<Vec<T>, Diagnostic>
where
    F: Fn(&BTreeMap<String, Value>) -> Result<T, Diagnostic>,
{
    let Some(value) = value else {
        return Err(bad_record());
    };
    let dict = dict_of(value)?;
    let mut out = Vec::new();
    for (key, entry) in dict {
        let inner = dict_of(entry)?;
        let id = required_text(inner, "id")?;
        if uuid_from_hex32(key.as_str())?.to_string() != id {
            return Err(fail(
                ErrorCode::InvalidId,
                "registry key differs from its record",
            ));
        }
        out.push(decode(inner)?);
    }
    Ok(out)
}

fn sdf_dictionary(entries: Vec<(String, sdf::Value)>) -> sdf::Value {
    sdf::Value::Dictionary(entries.into_iter().collect())
}

fn sdf_text(text: &str) -> sdf::Value {
    sdf::Value::String(text.to_owned())
}

/// Encodes the descriptor as the reserved `customLayerData` entry (text values and nested
/// dictionaries only). The caller validated the descriptor; a layer map that cannot encode is
/// refused instead of silently emptied.
pub fn descriptor_value(descriptor: &ProjectDescriptor) -> Result<sdf::Value, Diagnostic> {
    let mut sections = Vec::new();
    for section in &descriptor.sections {
        let key = registry_key(&section.id)?;
        sections.push((
            key,
            sdf_dictionary(vec![
                ("id".into(), sdf_text(&section.id)),
                ("code".into(), sdf_text(&section.code)),
                ("label".into(), sdf_text(&section.label)),
                ("modules".into(), sdf_text(&section.modules.join(","))),
            ]),
        ));
    }
    let mut zones = Vec::new();
    for zone in &descriptor.zones {
        let key = registry_key(&zone.id)?;
        zones.push((
            key,
            sdf_dictionary(vec![
                ("id".into(), sdf_text(&zone.id)),
                ("code".into(), sdf_text(&zone.code)),
                ("label".into(), sdf_text(&zone.label)),
            ]),
        ));
    }
    let mut connections = Vec::new();
    for connection in &descriptor.connections {
        let (scope, scope_id) = match &connection.scope {
            ScopeSelection::WholeProject => ("whole".to_owned(), String::new()),
            ScopeSelection::Section(id) => ("section".to_owned(), id.to_string()),
            ScopeSelection::Zone(id) => ("zone".to_owned(), id.to_string()),
        };
        let mut entries = vec![
            ("id".to_owned(), sdf_text(&connection.id.to_string())),
            (
                "sourceProject".to_owned(),
                sdf_text(&connection.source_project.to_string()),
            ),
            (
                "sourceWorkspace".to_owned(),
                sdf_text(&connection.source_workspace.to_string()),
            ),
            (
                "sourceRevision".to_owned(),
                sdf_text(&connection.source_revision.to_string()),
            ),
            ("manifest".to_owned(), sdf_text(&connection.manifest)),
            (
                "layers".to_owned(),
                sdf_text(
                    &beamcraft_model::descriptor::encode_layers(&connection.layers)
                        .map_err(|_| bad_record())?,
                ),
            ),
            ("scope".to_owned(), sdf_text(&scope)),
            ("scopeId".to_owned(), sdf_text(&scope_id)),
            (
                "alignment".to_owned(),
                sdf_text(match &connection.placement {
                    Some(_) => "placed",
                    None => "unknown",
                }),
            ),
            ("label".to_owned(), sdf_text(&connection.label)),
        ];
        if let Some(placement) = &connection.placement {
            let (translation, orientation) = encode_placement(placement);
            entries.push(("translationMm".to_owned(), sdf_text(&translation)));
            entries.push(("orientationXyzw".to_owned(), sdf_text(&orientation)));
        }
        connections.push((connection.id.uuid().hex32(), sdf_dictionary(entries)));
    }
    Ok(sdf_dictionary(vec![
        (
            "project".to_owned(),
            sdf_text(&descriptor.project.uuid().hex32()),
        ),
        ("code".to_owned(), sdf_text(&descriptor.metadata.code)),
        ("name".to_owned(), sdf_text(&descriptor.metadata.name)),
        (
            "organization".to_owned(),
            sdf_text(
                descriptor
                    .metadata
                    .organization
                    .as_deref()
                    .unwrap_or_default(),
            ),
        ),
        (
            "contract".to_owned(),
            sdf_text(&descriptor.contract.contract),
        ),
        (
            "vocabulary".to_owned(),
            sdf_text(&descriptor.contract.vocabulary),
        ),
        ("schema".to_owned(), sdf_text(&descriptor.contract.schema)),
        ("frame".to_owned(), sdf_text(&descriptor.frame)),
        ("sections".to_owned(), sdf_dictionary(sections)),
        ("zones".to_owned(), sdf_dictionary(zones)),
        ("connections".to_owned(), sdf_dictionary(connections)),
    ]))
}

/// The registry key of one record identity: the 32-hex-digit form of its UUIDv7.
fn registry_key(id: &str) -> Result<String, Diagnostic> {
    Ok(Uuid7::parse(id)
        .map_err(|_| fail(ErrorCode::InvalidId, "invalid registry identity"))?
        .hex32())
}

/// Rewrites the root layer bytes with the given descriptor record (or without one), preserving
/// every other `customLayerData` entry, the schema pin and the layer header.
pub fn write_root_descriptor(
    root_bytes: &[u8],
    descriptor: Option<&ProjectDescriptor>,
) -> Result<Vec<u8>, Diagnostic> {
    let text = std::str::from_utf8(root_bytes).map_err(|_| corrupt())?;
    let mut layer = openusd::usda::parse(text).map_err(|_| corrupt())?;
    let root = path("/")?;
    let mut dictionary = match layer
        .try_field(&root, "customLayerData")
        .map_err(|_| corrupt())?
    {
        Some(value) => match value.as_ref() {
            sdf::Value::Dictionary(dict) => dict.clone(),
            _ => return Err(corrupt()),
        },
        None => Default::default(),
    };
    match descriptor {
        Some(descriptor) => {
            descriptor.validate().map_err(model_fail)?;
            dictionary.insert(DESCRIPTOR_KEY.to_owned(), descriptor_value(descriptor)?);
        }
        None => {
            dictionary.remove(DESCRIPTOR_KEY);
        }
    }
    field(
        &mut layer,
        "/",
        "customLayerData",
        sdf::Value::Dictionary(dictionary),
    )?;
    serialize(&layer, crate::Budgets::default().layer_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_model::coordination::{SectionRecord, ZoneRecord};

    const PROJECT: &str = "019a7bf0-0000-7000-8000-00000000c001";
    const SECTION: &str = "019a7bf0-0000-7000-8000-00000000c101";
    const ZONE: &str = "019a7bf0-0000-7000-8000-00000000c201";
    const CONNECTION: &str = "019a7bf0-0000-7000-8000-00000000c301";
    const SOURCE: &str = "019a7bf0-0000-7000-8000-00000000c002";
    const SCHEMA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn sample() -> ProjectDescriptor {
        let mut descriptor =
            ProjectDescriptor::new(ProjectId::parse(PROJECT).unwrap(), SCHEMA).unwrap();
        descriptor
            .set_metadata(Some("2401-DEMO"), Some("Demo"), Some("ACME"))
            .unwrap();
        descriptor
            .put_section(SectionRecord {
                id: SECTION.to_owned(),
                code: "KM".to_owned(),
                label: "Steel".to_owned(),
                modules: vec!["steel".to_owned()],
            })
            .unwrap();
        descriptor
            .put_zone(ZoneRecord {
                id: ZONE.to_owned(),
                code: "K1".to_owned(),
                label: "K1".to_owned(),
            })
            .unwrap();
        descriptor
            .put_connection(ConnectionRecord {
                id: ConnectionId::parse(CONNECTION).unwrap(),
                source_project: ProjectId::parse(SOURCE).unwrap(),
                source_workspace: WorkspaceId::parse(SOURCE).unwrap(),
                source_revision: RevisionId::parse(SOURCE).unwrap(),
                manifest: SCHEMA.to_owned(),
                layers: vec![("authored.usda".to_owned(), SCHEMA.to_owned())],
                scope: ScopeSelection::Section(SectionId::parse(SECTION).unwrap()),
                placement: Some(ConnectionPlacement::IDENTITY),
                label: "steel source".to_owned(),
            })
            .unwrap();
        descriptor
    }

    fn metadata_with(value: Value) -> BTreeMap<String, Value> {
        let mut custom = BTreeMap::new();
        custom.insert("bimSchemaDigest".to_owned(), Value::Text(SCHEMA.to_owned()));
        custom.insert(DESCRIPTOR_KEY.to_owned(), value);
        let mut metadata = BTreeMap::new();
        metadata.insert("customLayerData".to_owned(), Value::Dictionary(custom));
        metadata
    }

    /// A descriptor value through one real usda round trip, as the stored bytes see it.
    fn round_tripped(descriptor: &ProjectDescriptor) -> Vec<u8> {
        let written = write_root_descriptor(EMPTY_ROOT, Some(descriptor)).unwrap();
        std::str::from_utf8(&written)
            .unwrap()
            .to_string()
            .into_bytes()
    }

    const EMPTY_ROOT: &[u8] = b"#usda 1.0\n(\n    customLayerData = {\n        string bimSchemaDigest = \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"\n        string bimProjectId = \"019a7bf0-0000-7000-8000-00000000c001\"\n    }\n    defaultPrim = \"World\"\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n)\n";

    #[test]
    fn the_descriptor_round_trips_through_real_layer_bytes() {
        let descriptor = sample();
        let written = round_tripped(&descriptor);
        let layer = openusd::usda::parse(std::str::from_utf8(&written).unwrap()).unwrap();
        let value = layer
            .try_field(&sdf::Path::abs_root(), "customLayerData")
            .unwrap()
            .unwrap();
        let sdf::Value::Dictionary(custom) = value.as_ref() else {
            panic!("no customLayerData");
        };
        // The other records survive the rewrite.
        assert!(custom.get("bimProjectId").is_some());
        assert!(custom.get("bimSchemaDigest").is_some());
        let record = custom.get(DESCRIPTOR_KEY).unwrap().clone();
        let decoded = descriptor_of(&metadata_with(flatten(record)))
            .unwrap()
            .unwrap();
        assert_eq!(decoded, descriptor);
    }

    #[test]
    fn a_layer_without_a_descriptor_reads_as_none() {
        let mut metadata = BTreeMap::new();
        let mut custom = BTreeMap::new();
        custom.insert("other".to_owned(), Value::Text("x".to_owned()));
        metadata.insert("customLayerData".to_owned(), Value::Dictionary(custom));
        assert!(descriptor_of(&metadata).unwrap().is_none());
        let mut empty = BTreeMap::new();
        empty.insert(
            "customLayerData".to_owned(),
            Value::Dictionary(BTreeMap::new()),
        );
        assert!(descriptor_of(&empty).unwrap().is_none());
    }

    #[test]
    fn a_malformed_record_is_a_typed_refusal_not_none() {
        let mut metadata = metadata_with(Value::Text("garbage".to_owned()));
        assert_eq!(
            descriptor_of(&metadata).unwrap_err().code,
            ErrorCode::IntegrityError
        );
        metadata = metadata_with(Value::Dictionary(BTreeMap::from([(
            "project".to_owned(),
            Value::Text("019a7bf0000070008000000000c001x".to_owned()),
        )])));
        // A truncated identity is an id refusal.
        let error = descriptor_of(&metadata).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidId);
    }

    #[test]
    fn removing_the_record_leaves_the_rest_of_the_layer() {
        let descriptor = sample();
        let with = write_root_descriptor(EMPTY_ROOT, Some(&descriptor)).unwrap();
        let without = write_root_descriptor(&with, None).unwrap();
        let text = std::str::from_utf8(&without).unwrap();
        assert!(!text.contains(DESCRIPTOR_KEY));
        assert!(text.contains("bimProjectId"));
        let layer = openusd::usda::parse(text).unwrap();
        assert!(
            layer
                .try_field(&sdf::Path::abs_root(), "customLayerData")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn an_unknown_alignment_record_stays_absent_in_the_placement() {
        let mut descriptor = sample();
        let mut connection = descriptor.connections.remove(0);
        connection.placement = None;
        descriptor.put_connection(connection).unwrap();
        let written = round_tripped(&descriptor);
        let layer = openusd::usda::parse(std::str::from_utf8(&written).unwrap()).unwrap();
        let value = layer
            .try_field(&sdf::Path::abs_root(), "customLayerData")
            .unwrap()
            .unwrap();
        let sdf::Value::Dictionary(custom) = value.as_ref() else {
            panic!()
        };
        let sdf::Value::Dictionary(record) = custom.get(DESCRIPTOR_KEY).unwrap().clone() else {
            panic!()
        };
        let Some(sdf::Value::Dictionary(connections)) = record.get("connections") else {
            panic!()
        };
        let entry = connections.values().next().unwrap().clone();
        let sdf::Value::Dictionary(connection) = entry else {
            panic!()
        };
        assert_eq!(
            connection.get("alignment").cloned(),
            Some(sdf::Value::String("unknown".to_owned()))
        );
        assert!(!connection.contains_key("translationMm"));
        // And it decodes back to an explicit unknown alignment.
        let decoded = descriptor_of(&metadata_with(flatten(sdf::Value::Dictionary(record))))
            .unwrap()
            .unwrap();
        assert!(decoded.connections[0].placement.is_none());
    }

    /// Flattens a decoded sdf dictionary into typed values (test helper: strings only here).
    fn flatten(value: sdf::Value) -> Value {
        match value {
            sdf::Value::String(s) => Value::Text(s),
            sdf::Value::Dictionary(dict) => {
                Value::Dictionary(dict.into_iter().map(|(k, v)| (k, flatten(v))).collect())
            }
            other => panic!("non-string {other:?}"),
        }
    }
}
