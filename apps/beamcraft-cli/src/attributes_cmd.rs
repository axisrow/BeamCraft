// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! I04 typed definition and value commands over the existing revision/undo protocol.
use beamcraft_cli::{
    attributes::{self, AttributeCommand, AttributeContext, Attributes},
    plate::PlateError,
};
use beamcraft_host::{
    ids::IdSource,
    revision::{NoFault, RevisionStore},
    storage::BlobStore,
};
use beamcraft_json::Json;
use beamcraft_model::{
    attribute::{AttributeDefinition, AttributeType, AttributeValue, MAX_VALUES},
    error::ErrorCode,
    id::ElementId,
};
use std::{collections::BTreeMap, ffi::OsString};
fn invalid(message: &'static str) -> PlateError {
    PlateError {
        code: ErrorCode::InvalidArgument,
        message,
        rule_id: None,
    }
}
pub fn handles(c: &OsString) -> bool {
    [
        "define-attribute",
        "list-attributes",
        "set-attributes",
        "inspect-attributes",
        "remove-attributes",
    ]
    .iter()
    .any(|s| c == s)
}
pub fn execute(args: &[OsString]) -> (Json, u8) {
    crate::report::report(run(args))
}
pub(crate) fn definition_json(d: &AttributeDefinition) -> Json {
    Json::object([
        ("id", Json::text(d.id.to_string())),
        ("name", Json::text(&d.name)),
        ("type", Json::text(d.kind.token())),
        ("unit", Json::text(&d.unit)),
        (
            "choices",
            Json::Array(d.choices.iter().map(Json::text).collect()),
        ),
        ("sets", Json::Array(d.sets.iter().map(Json::text).collect())),
    ])
}
pub(crate) fn raw_json(v: &beamcraft_layer_engine::Value) -> Result<Json, PlateError> {
    use beamcraft_layer_engine::Value as V;
    Ok(match v {
        V::Block => Json::Null,
        V::Bool(v) => Json::Bool(*v),
        V::Int(v) => Json::Number(v.to_string()),
        V::Uint(v) => Json::uint(*v),
        V::Number(v) => crate::plate_cmd::num(*v)?,
        V::Text(v) => Json::text(v),
        V::Array(v) => Json::Array(v.iter().map(raw_json).collect::<Result<Vec<_>, _>>()?),
        V::Dictionary(v) => Json::Object(
            v.iter()
                .map(|(k, v)| Ok((k.clone(), raw_json(v)?)))
                .collect::<Result<_, PlateError>>()?,
        ),
    })
}
fn inspection(attrs: &Attributes, id: ElementId) -> Result<Json, PlateError> {
    let e = attrs.elements.get(&id).ok_or(PlateError {
        code: ErrorCode::NotFound,
        message: "attribute target not found",
        rule_id: None,
    })?;
    let properties = e
        .properties
        .iter()
        .map(|p| {
            let id = beamcraft_layer_engine::authored::attribute::property_id(&p.name);
            let def = id.and_then(|id| attrs.definitions.get(&id));
            Ok(Json::object([
                ("property", Json::text(&p.name)),
                (
                    "attribute_id",
                    id.map_or(Json::Null, |id| Json::text(id.to_string())),
                ),
                ("known", Json::Bool(def.is_some())),
                ("definition", def.map_or(Json::Null, definition_json)),
                ("usd_type", Json::text(&p.type_name)),
                (
                    "state",
                    Json::text(match p.value {
                        None => "unset",
                        Some(beamcraft_layer_engine::Value::Block) => "blocked",
                        _ => "value",
                    }),
                ),
                (
                    "value",
                    p.value
                        .as_ref()
                        .map(raw_json)
                        .transpose()?
                        .unwrap_or(Json::Null),
                ),
            ]))
        })
        .collect::<Result<Vec<_>, PlateError>>()?;
    Ok(Json::object([
        ("outcome", Json::text("attributes")),
        ("element_id", Json::text(id.to_string())),
        ("attributes", Json::Array(properties)),
        (
            "revision",
            attrs
                .revision
                .as_ref()
                .map_or(Json::Null, crate::plate_cmd::revision_json),
        ),
    ]))
}
fn scalar(j: &Json, d: &AttributeDefinition) -> Result<AttributeValue, PlateError> {
    let j = if let Json::Object(_) = j {
        j.closed(&["value", "unit"])
            .map_err(|_| invalid("unit value must contain value and unit"))?;
        if j.get("unit")
            .and_then(Json::string)
            .map_err(|_| invalid("unit must be text"))?
            != d.unit
            || !matches!(d.kind, AttributeType::Number | AttributeType::Integer)
        {
            return Err(invalid("attribute unit does not match its definition"));
        }
        j.get("value")
            .map_err(|_| invalid("unit value is missing"))?
    } else {
        j
    };
    let value = match d.kind {
        AttributeType::Text | AttributeType::Choice => AttributeValue::Text(
            j.string()
                .map_err(|_| invalid("attribute requires text"))?
                .to_owned(),
        ),
        AttributeType::Number => AttributeValue::Number(
            j.float()
                .map_err(|_| invalid("attribute requires a number"))?,
        ),
        AttributeType::Integer => {
            let Json::Number(s) = j else {
                return Err(invalid("attribute requires an exact integer"));
            };
            AttributeValue::Integer(
                s.parse()
                    .map_err(|_| invalid("attribute requires an exact int64"))?,
            )
        }
        AttributeType::Boolean => AttributeValue::Boolean(
            j.boolean()
                .map_err(|_| invalid("attribute requires a boolean"))?,
        ),
    };
    d.admits(&value)?;
    Ok(value)
}
fn run(args: &[OsString]) -> Result<Json, PlateError> {
    let args = args
        .iter()
        .map(|a| {
            a.to_str()
                .filter(|s| s.len() <= 4096)
                .ok_or_else(|| invalid("argument must be bounded UTF-8"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (command, tail) = args
        .split_first()
        .ok_or_else(|| invalid("missing command"))?;
    if !tail.len().is_multiple_of(2) || tail.len() > 48 {
        return Err(invalid("options must be bounded KEY VALUE pairs"));
    }
    let own: &[&str] = match *command {
        "define-attribute" => &[
            "--store",
            "--new-id",
            "--name",
            "--type",
            "--unit",
            "--choices",
            "--sets",
        ],
        "list-attributes" => &["--store", "--offset", "--limit", "--set"],
        "inspect-attributes" => &["--store", "--id"],
        "set-attributes" => &["--store", "--id", "--values"],
        "remove-attributes" => &["--store", "--id", "--attributes"],
        _ => return Err(invalid("unknown attribute command")),
    };
    let mutation = matches!(
        *command,
        "define-attribute" | "set-attributes" | "remove-attributes"
    );
    let mut options = BTreeMap::new();
    for [k, v] in tail.as_chunks::<2>().0 {
        if !(own.contains(k) || (mutation && crate::command_context::CONTEXT_OPTIONS.contains(k)))
            || options.insert(*k, *v).is_some()
        {
            return Err(invalid("unknown or repeated attribute option"));
        }
    }
    let given = |k: &str| options.get(k).copied();
    let need = |k: &str| given(k).ok_or_else(|| invalid("required attribute option is missing"));
    let store = RevisionStore::new(BlobStore::new(need("--store")?).map_err(|e| PlateError {
        code: e.code(),
        message: "store root is not usable",
        rule_id: None,
    })?);
    let attrs = attributes::read(&store)?;
    if *command == "inspect-attributes" {
        return inspection(
            &attrs,
            ElementId::parse(need("--id")?).map_err(crate::plate_cmd::id_error)?,
        );
    }
    if *command == "list-attributes" {
        let offset = given("--offset")
            .unwrap_or("0")
            .parse::<usize>()
            .map_err(|_| invalid("offset must be an integer"))?;
        let limit = given("--limit")
            .unwrap_or("16")
            .parse::<usize>()
            .map_err(|_| invalid("limit must be an integer"))?;
        if !(1..=64).contains(&limit) {
            return Err(invalid("limit must be 1..64"));
        }
        let defs = attrs
            .definitions
            .values()
            .filter(|d| given("--set").is_none_or(|s| d.sets.iter().any(|set| set == s)))
            .collect::<Vec<_>>();
        let page = defs
            .iter()
            .skip(offset)
            .take(limit)
            .map(|d| definition_json(d))
            .collect();
        return Ok(Json::object([
            ("outcome", Json::text("attribute_definitions")),
            ("definitions", Json::Array(page)),
            (
                "next_offset",
                if offset.saturating_add(limit) < defs.len() {
                    Json::uint(offset.saturating_add(limit) as u64)
                } else {
                    Json::Null
                },
            ),
        ]));
    }
    let _claim = crate::plate_cmd::check_project_option(&store, &options)?;
    let mut source = IdSource::system();
    let mut fresh = || {
        source
            .next_id()
            .map_err(|_| invalid("attribute identity unavailable"))
    };
    let ctx = crate::command_context::parse(&options, &mut fresh)?;
    let context = AttributeContext {
        op_id: ctx.op_id,
        revision_id: ctx.revision_id,
        actor: ctx.actor,
        project: ctx.project,
        workspace: ctx.workspace,
    };
    let command = if *command == "define-attribute" {
        let id = match given("--new-id") {
            Some(id) => ElementId::parse(id).map_err(crate::plate_cmd::id_error)?,
            None => ElementId::from_uuid(fresh()?),
        };
        let kind = AttributeType::parse(need("--type")?)
            .ok_or_else(|| invalid("unknown attribute type"))?;
        let choices = given("--choices")
            .map(|s| s.split('|').take(129).map(str::to_owned).collect())
            .unwrap_or_default();
        let sets = given("--sets")
            .map(|s| s.split(',').take(17).map(str::to_owned).collect())
            .unwrap_or_default();
        AttributeCommand::Define(AttributeDefinition {
            id,
            name: need("--name")?.to_owned(),
            kind,
            unit: given("--unit").unwrap_or("").to_owned(),
            choices,
            sets,
        })
    } else {
        let id = ElementId::parse(need("--id")?).map_err(crate::plate_cmd::id_error)?;
        let mut values = BTreeMap::new();
        if *command == "set-attributes" {
            let bytes = beamcraft_cli::source_file::read_file(need("--values")?, 65536).map_err(
                |code| PlateError {
                    code,
                    message: "attribute input file is not usable",
                    rule_id: None,
                },
            )?;
            let j = beamcraft_json::decode(&bytes, &beamcraft_json::Limits::DEFAULT)
                .map_err(|_| invalid("attribute values must be a JSON object"))?;
            let Json::Object(map) = j else {
                return Err(invalid("attribute values must be a JSON object"));
            };
            if map.len() > MAX_VALUES {
                return Err(invalid("too many attribute values"));
            }
            for (name, value) in map {
                let def = attributes::definition(&attrs, &name)?;
                if values.insert(def.id, Some(scalar(&value, def)?)).is_some() {
                    return Err(invalid("attribute is specified more than once"));
                }
            }
        } else {
            for name in need("--attributes")?.split(',').take(MAX_VALUES + 1) {
                let key = ElementId::parse(name)
                    .ok()
                    .or_else(|| attributes::definition(&attrs, name).ok().map(|d| d.id))
                    .ok_or_else(|| invalid("attribute to remove is unknown"))?;
                if values.insert(key, None).is_some() {
                    return Err(invalid("attribute is specified more than once"));
                }
            }
        }
        AttributeCommand::Patch { id, values }
    };
    let result = attributes::apply(&store, &context, &command, &mut NoFault)?;
    let mut reply = Json::object([
        (
            "outcome",
            Json::text(if result.replayed {
                "attributes_replayed"
            } else if matches!(command, AttributeCommand::Define(_)) {
                "attribute_defined"
            } else {
                "attributes_updated"
            }),
        ),
        (
            "revision",
            crate::plate_cmd::revision_json(&result.revision),
        ),
    ]);
    if let (AttributeCommand::Define(d), Json::Object(fields)) = (&command, &mut reply) {
        fields.insert("definition".into(), definition_json(d));
    }
    Ok(reply)
}
