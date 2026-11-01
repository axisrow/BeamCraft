// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! I04 headless commands: `attribute-define`, `attribute-undefine`, `attribute-set`,
//! `attribute-clear`, `attribute-get` and `attributes`. They call the engine API through
//! `beamcraft_cli::attribute` and print one JSON object with the same error shape and exit
//! codes as the plate commands.

use crate::command_context::{self, CONTEXT_OPTIONS};
use crate::plate_cmd::{check_project_option, footprint_json, id_error, num, revision_json};
use beamcraft_cli::attribute::{
    AttrContext, Scalar, ValueEntry, clear_attribute, define_attribute, definition_of, parse_value,
    set_attribute, undefine_attribute, values_of,
};
use beamcraft_host::{
    ids::IdSource,
    revision::{NoFault, RevisionStore},
    storage::BlobStore,
};
use beamcraft_json::Json;
use beamcraft_model::attribute::{AttrDef, AttrType, AttrValue, DefineAttribute};
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

fn not_defined() -> Refusal {
    Refusal {
        rule_id: None,
        code: ErrorCode::NotFound,
        message: "the attribute is not defined",
    }
}

/// Runs one attribute command and returns the JSON result and the process exit code.
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
    let reads = matches!(*command, "attributes" | "attribute-get");
    let own: &[&str] = match *command {
        "attribute-define" => &[
            "--store",
            "--name",
            "--type",
            "--unit",
            "--options",
            "--set",
        ],
        "attribute-undefine" => &["--store", "--name"],
        "attribute-set" => &["--store", "--element", "--name", "--value"],
        "attribute-clear" => &["--store", "--element", "--name"],
        "attribute-get" => &["--store", "--element"],
        "attributes" => &["--store"],
        _ => return Err(invalid("unknown command")),
    };
    if !tail.len().is_multiple_of(2) || tail.len() > 40 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        let allowed = own.contains(key) || (!reads && CONTEXT_OPTIONS.contains(key));
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
    if *command == "attributes" {
        let defs = beamcraft_cli::attribute::definitions(&store)?;
        return Ok(Json::object([
            ("outcome", Json::text("attributes")),
            (
                "attributes",
                Json::Array(defs.iter().map(def_json).collect()),
            ),
        ]));
    }
    if *command == "attribute-get" {
        let element = element_of(&options)?;
        let entries = values_of(&store, &element)?;
        let entries = entries.iter().map(value_json).collect::<Run<Vec<_>>>()?;
        return Ok(Json::object([
            ("outcome", Json::text("attribute_get")),
            ("element_id", Json::text(element.to_string())),
            ("attributes", Json::Array(entries)),
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
    let context = AttrContext {
        op_id: common.op_id,
        revision_id: common.revision_id,
        actor: common.actor,
        project: common.project,
        workspace: common.workspace,
    };
    let (subject, change) = match *command {
        "attribute-define" => {
            let name = given("--name")
                .ok_or_else(|| invalid("--name is required"))?
                .to_owned();
            let ty = AttrType::parse(given("--type").ok_or_else(|| invalid("--type is required"))?)
                .ok_or_else(|| {
                    invalid("--type is one of text, number, integer, boolean, choice")
                })?;
            let unit = given("--unit").map(str::to_owned);
            let choices = match (ty, given("--options")) {
                (AttrType::Choice, Some(text)) => {
                    Some(text.split(',').map(str::to_owned).collect::<Vec<String>>())
                }
                (AttrType::Choice, None) => {
                    return Err(invalid("a choice declares --options a,b,c"));
                }
                (_, Some(_)) => return Err(invalid("only a choice carries options")),
                (_, None) => None,
            };
            let command = DefineAttribute {
                def: AttrDef {
                    name,
                    ty,
                    unit,
                    options: choices.unwrap_or_default(),
                    set: given("--set").map(str::to_owned),
                },
            };
            let subject = def_json(&command.def);
            let change = define_attribute(&store, &context, &command, &mut NoFault)?;
            (subject, change)
        }
        "attribute-undefine" => {
            let name = given("--name")
                .ok_or_else(|| invalid("--name is required"))?
                .to_owned();
            let command = beamcraft_model::attribute::UndefineAttribute { name: name.clone() };
            let subject = Json::object([("name", Json::text(name))]);
            let change = undefine_attribute(&store, &context, &command, &mut NoFault)?;
            (subject, change)
        }
        "attribute-set" => {
            let element = element_of(&options)?;
            let name = given("--name")
                .ok_or_else(|| invalid("--name is required"))?
                .to_owned();
            let def = definition_of(&store, &name)?.ok_or_else(not_defined)?;
            let raw = given("--value").ok_or_else(|| invalid("--value is required"))?;
            let value = parse_value(def.ty, raw)?;
            let command = beamcraft_model::attribute::SetAttribute {
                target_id: element,
                name: name.clone(),
                value: value.clone(),
            };
            let subject = Json::object([
                ("element_id", Json::text(element.to_string())),
                ("name", Json::text(name)),
                ("value", attr_value_json(&value)?),
            ]);
            let change = set_attribute(&store, &context, &command, &mut NoFault)?;
            (subject, change)
        }
        "attribute-clear" => {
            let element = element_of(&options)?;
            let name = given("--name")
                .ok_or_else(|| invalid("--name is required"))?
                .to_owned();
            let command = beamcraft_model::attribute::ClearAttribute {
                target_id: element,
                name: name.clone(),
            };
            let subject = Json::object([
                ("element_id", Json::text(element.to_string())),
                ("name", Json::text(name)),
            ]);
            let change = clear_attribute(&store, &context, &command, &mut NoFault)?;
            (subject, change)
        }
        _ => return Err(invalid("unknown command")),
    };
    // The reads returned earlier, so the command is one of the four `attribute-` mutations.
    let base = format!(
        "attribute_{}",
        command.strip_prefix("attribute-").unwrap_or(command)
    );
    change_json(&base, context.op_id, subject, &change)
}

fn element_of(options: &BTreeMap<&str, &str>) -> Run<ElementId> {
    let text = options
        .get("--element")
        .ok_or_else(|| invalid("--element is required"))?;
    ElementId::parse(text).map_err(id_error)
}

/// The JSON of one definition.
pub fn def_json(def: &AttrDef) -> Json {
    Json::object([
        ("name", Json::text(def.name.clone())),
        ("type", Json::text(def.ty.as_str())),
        (
            "unit",
            def.unit
                .as_ref()
                .map(|u| Json::text(u.clone()))
                .unwrap_or(Json::Null),
        ),
        (
            "options",
            if def.ty == AttrType::Choice {
                Json::Array(def.options.iter().map(|o| Json::text(o.clone())).collect())
            } else {
                Json::Array(vec![])
            },
        ),
        (
            "set",
            def.set
                .as_ref()
                .map(|s| Json::text(s.clone()))
                .unwrap_or(Json::Null),
        ),
    ])
}

fn scalar_json(value: &Scalar) -> Run<Json> {
    Ok(match value {
        Scalar::Text(t) => Json::text(t.clone()),
        Scalar::Number(n) => num(*n)?,
        Scalar::Integer(i) => num(*i as f64)?,
        Scalar::Boolean(b) => Json::Bool(*b),
    })
}

fn attr_value_json(value: &AttrValue) -> Run<Json> {
    Ok(match value {
        AttrValue::Text(t) | AttrValue::Choice(t) => Json::text(t.clone()),
        AttrValue::Number(n) => num(*n)?,
        AttrValue::Integer(i) => num(f64::from(*i))?,
        AttrValue::Boolean(b) => Json::Bool(*b),
    })
}

fn value_json(entry: &ValueEntry) -> Run<Json> {
    Ok(match entry {
        ValueEntry::Defined { def, value } => {
            let value = attr_value_json(value)?;
            match &def.unit {
                Some(unit) => Json::object([
                    ("name", Json::text(def.name.clone())),
                    ("defined", Json::Bool(true)),
                    ("type", Json::text(def.ty.as_str())),
                    ("value", value),
                    ("unit", Json::text(unit.clone())),
                ]),
                None => Json::object([
                    ("name", Json::text(def.name.clone())),
                    ("defined", Json::Bool(true)),
                    ("type", Json::text(def.ty.as_str())),
                    ("value", value),
                ]),
            }
        }
        ValueEntry::Unknown(unknown) => Json::object([
            ("name", Json::text(unknown.name.clone())),
            ("defined", Json::Bool(false)),
            ("usd_type", Json::text(unknown.type_name.clone())),
            (
                "value",
                match &unknown.value {
                    Some(scalar) => scalar_json(scalar)?,
                    None => Json::Null,
                },
            ),
        ]),
    })
}

fn change_json(
    base: &str,
    op_id: OpId,
    subject: Json,
    change: &beamcraft_cli::attribute::AttrChange,
) -> Run<Json> {
    let outcome = if change.replayed {
        format!("{base}_replayed")
    } else {
        base.to_owned()
    };
    Ok(Json::object([
        ("outcome", Json::text(outcome)),
        ("op_id", Json::text(op_id.to_string())),
        ("subject", subject),
        (
            "changed_layers",
            Json::Array(change.changed_layers.iter().map(Json::text).collect()),
        ),
        ("revision", revision_json(&change.revision)),
        ("footprint", footprint_json(change.footprint.as_ref())),
    ]))
}
