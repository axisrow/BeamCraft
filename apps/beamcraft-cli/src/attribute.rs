// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! I04 adapter: the attribute schema and value commands committed as the next revision, plus
//! the reads behind `attributes` and `attribute-get`.
//!
//! Definitions are mutable schema metadata under `/World/Definitions/Attributes`, never
//! elements; values are `bim:attr:<name>` properties of any element. A value may outlive its
//! definition: the reader reports it as an unknown attribute next to the defined ones, and no
//! write path rewrites a prim's property set, so an unknown attribute is shown, never dropped.
//!
//! Honesty limits: replay of an `op_id` is decided from the head model, as in `update-element`
//! (the operation is a replay when the head already holds its requested state); a later
//! contradicting edit of the same state turns a retry into `OP_ID_REUSE`. One implicit
//! workspace, in-process layer engine, as in `level`. Attribute changes are not rebaseable yet:
//! the rebase property mapping refuses them as unsupported deltas.

use crate::assembly::read_prims;
use crate::plate::{
    AUTHORED_LAYER, Base, DERIVED_LAYER, PlateError, RevisionSummary, check_footprint, control,
    digest_of, host_uuid, integrity, load_base, open_base, prior_op_with, property, replay_summary,
    summarize, text,
};
use crate::publication::{PublishRequest, changed_layers, publish_revision};
use beamcraft_engine::attribute::{ElementValues, Schema};
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::transaction::PriorOp;
use beamcraft_host::revision::{CommitHook, CommitOutcome, RevisionStore, Uuid7};
use beamcraft_layer_engine::authored::{
    ATTR_SCHEMA_PATH, Author, Change, EditTargets, Prepare, schema_identity, validate_layers,
};
use beamcraft_layer_engine::{Control, OpenRequest, Reader, Value};
use beamcraft_model::attribute::{
    AttrDef, AttrType, AttrValue, ClearAttribute, DefineAttribute, SetAttribute, UndefineAttribute,
};
use beamcraft_model::command::CommandError;
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId};
use std::collections::BTreeMap;

/// Identities of one I04 call beyond the command itself.
#[derive(Debug, Clone)]
pub struct AttrContext {
    /// The logical operation.
    pub op_id: OpId,
    /// Identity of the revision this call creates.
    pub revision_id: RevisionId,
    /// The acting user (trusted context).
    pub actor: Uuid7,
    /// Project of a store that has no revision yet; its first revision records it.
    pub project: ProjectId,
    /// Workspace of a store that has no revision yet.
    pub workspace: WorkspaceId,
}

/// A raw stored attribute the schema does not explain: a foreign value or one that outlived its
/// definition. It is reported, never dropped and never edited as a typed value.
#[derive(Debug, Clone, PartialEq)]
pub struct UnknownValue {
    /// The attribute name without the `bim:attr:` prefix.
    pub name: String,
    /// The stored USD type name.
    pub type_name: String,
    /// The stored scalar, when the property carries one this reader renders.
    pub value: Option<Scalar>,
}

/// A stored scalar of an unknown attribute.
#[derive(Debug, Clone, PartialEq)]
pub enum Scalar {
    /// Text or token.
    Text(String),
    /// Number.
    Number(f64),
    /// Integer.
    Integer(i64),
    /// Boolean.
    Boolean(bool),
}

/// One value of one element: typed against its definition, or unknown.
#[derive(Debug, Clone, PartialEq)]
pub enum ValueEntry {
    /// The attribute is defined and the stored value matches its type.
    Defined {
        /// The definition.
        def: AttrDef,
        /// The stored value.
        value: AttrValue,
    },
    /// The attribute has no definition or the stored type does not match it.
    Unknown(UnknownValue),
}

/// The decoded attribute state of the head: the schema, the typed values and the unknown ones.
#[derive(Debug, Clone, Default)]
pub struct AttrModel {
    /// The attribute schema.
    pub schema: Schema,
    /// The typed values and the elements they live on.
    pub values: ElementValues,
    /// The unknown values by element identity text.
    pub unknown: BTreeMap<String, Vec<UnknownValue>>,
}

/// Result of one I04 mutation.
#[derive(Debug, Clone, PartialEq)]
pub struct AttrChange {
    /// True when the same operation was already committed and nothing was written.
    pub replayed: bool,
    /// The revision that holds the result.
    pub revision: RevisionSummary,
    /// The declared footprint; `None` for a replay.
    pub footprint: Option<Footprint>,
    /// Names of the layers whose bytes changed (empty for a replay).
    pub changed_layers: Vec<String>,
}

/// The stored scalar of a property, when it carries one this reader renders.
fn scalar(value: Option<&Value>) -> Option<Scalar> {
    match value? {
        Value::Text(t) => Some(Scalar::Text(t.clone())),
        Value::Number(n) => Some(Scalar::Number(*n)),
        Value::Int(i) => Some(Scalar::Integer(*i)),
        Value::Uint(u) => i64::try_from(*u).ok().map(Scalar::Integer),
        Value::Bool(b) => Some(Scalar::Boolean(*b)),
        _ => None,
    }
}

/// The typed value of a scalar under a definition, when the type matches.
fn typed(ty: AttrType, value: &Scalar) -> Option<AttrValue> {
    match (ty, value) {
        (AttrType::Text, Scalar::Text(t)) => Some(AttrValue::Text(t.clone())),
        (AttrType::Choice, Scalar::Text(t)) => Some(AttrValue::Choice(t.clone())),
        (AttrType::Number, Scalar::Number(n)) => Some(AttrValue::Number(*n)),
        (AttrType::Integer, Scalar::Integer(i)) => {
            Some(AttrValue::Integer(i32::try_from(*i).ok()?))
        }
        (AttrType::Boolean, Scalar::Boolean(b)) => Some(AttrValue::Boolean(*b)),
        _ => None,
    }
}

/// The closest typed view of a raw stored scalar, so an unknown value stays clearable.
fn presence(value: &Scalar) -> Option<AttrValue> {
    match value {
        Scalar::Text(t) => Some(AttrValue::Text(t.clone())),
        Scalar::Number(n) => Some(AttrValue::Number(*n)),
        Scalar::Integer(i) => Some(AttrValue::Integer(i32::try_from(*i).ok()?)),
        Scalar::Boolean(b) => Some(AttrValue::Boolean(*b)),
    }
}

/// The attribute state of a pinned set of prims. Foreign or unsupported content stays inert:
/// its identities count as used and its attributes are reported as unknown values only.
pub(crate) fn attr_model_of(
    request: &OpenRequest,
    control: &Control,
) -> Result<AttrModel, PlateError> {
    // Stored bytes are verified, not interpreted: the structural rules run before the attributes
    // are read, so a damaged graph is refused rather than reported or replayed.
    validate_layers(request.clone(), schema_identity()?, control)?;
    let mut reader = Reader::default();
    let opened = reader.open(request.clone(), control)?;
    let prims = read_prims(&reader, &opened.context, control)?;
    let scope = format!("{ATTR_SCHEMA_PATH}/");
    let mut model = AttrModel::default();
    for prim in &prims {
        if let Some(id) = prim
            .element_id
            .as_deref()
            .and_then(|id| ElementId::parse(id).ok())
        {
            model.values.insert_element(id);
        }
        if !prim.path.starts_with(&scope) {
            continue;
        }
        if prim.sources.iter().any(|s| !s.owned) {
            continue;
        }
        let name = prim
            .path
            .strip_prefix(&scope)
            .ok_or_else(|| integrity("attribute definition is not one segment below the scope"))?;
        let ty = property(prim, "bim:attr:type")
            .and_then(text)
            .and_then(AttrType::parse)
            .ok_or_else(|| integrity("attribute definition has no type"))?;
        let options = property(prim, "bim:attr:options")
            .and_then(|v| match v {
                Value::Array(items) => items
                    .iter()
                    .map(|item| match item {
                        Value::Text(option) => Some(option.clone()),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>(),
                _ => None,
            })
            .unwrap_or_default();
        model.schema.insert(AttrDef {
            name: name.to_owned(),
            ty,
            unit: property(prim, "bim:attr:unit")
                .and_then(text)
                .map(str::to_owned),
            options,
            set: property(prim, "bim:attr:set")
                .and_then(text)
                .map(str::to_owned),
        });
    }
    for prim in &prims {
        let Some(id) = prim
            .element_id
            .as_deref()
            .and_then(|id| ElementId::parse(id).ok())
        else {
            continue;
        };
        let owned = prim.sources.iter().all(|s| s.owned);
        for prop in &prim.properties {
            let Some(name) = prop.name.strip_prefix("bim:attr:") else {
                continue;
            };
            let raw = scalar(prop.value.as_ref());
            let typed = if owned {
                model
                    .schema
                    .get(name)
                    .and_then(|def| raw.as_ref().and_then(|r| typed(def.ty, r)))
            } else {
                None
            };
            match typed {
                Some(value) => model.values.insert_value(id, name, value),
                None => {
                    // An unknown value still occupies its element: record a best-effort typed
                    // presence so a clear finds it, and report it as unknown.
                    if owned && let Some(presence) = raw.as_ref().and_then(presence) {
                        model.values.insert_value(id, name, presence);
                    }
                    model
                        .unknown
                        .entry(id.to_string())
                        .or_default()
                        .push(UnknownValue {
                            name: name.to_owned(),
                            type_name: prop.type_name.clone(),
                            value: raw,
                        });
                }
            }
        }
    }
    Ok(model)
}

/// The attribute state of the verified head (empty for a store without a revision).
pub(crate) fn attr_model(store: &RevisionStore) -> Result<AttrModel, PlateError> {
    let control = control();
    let revision = match store.open() {
        Ok(revision) => revision,
        Err(beamcraft_host::revision::RevisionError::NoRevision) => return Ok(AttrModel::default()),
        Err(error) => return Err(error.into()),
    };
    let content = revision.manifest().content();
    let damaged = |_| integrity("head manifest identity is not a UUIDv7");
    let project = ProjectId::parse(content.project().as_str()).map_err(damaged)?;
    let workspace = WorkspaceId::parse(content.workspace().as_str()).map_err(damaged)?;
    let (_, base) = load_base(store, &project, &workspace)?;
    attr_model_of(&open_base(&base)?, &control)
}

/// Every definition of the head, in name order.
pub fn definitions(store: &RevisionStore) -> Result<Vec<AttrDef>, PlateError> {
    Ok(attr_model(store)?.schema.iter().cloned().collect())
}

/// The values of one element, defined and unknown, in name order.
pub fn values_of(
    store: &RevisionStore,
    element: &ElementId,
) -> Result<Vec<ValueEntry>, PlateError> {
    let model = attr_model(store)?;
    let id_text = element.to_string();
    let mut defined: Vec<ValueEntry> = model
        .schema
        .iter()
        .filter_map(|def| {
            let value = model.values.value_of(element, &def.name)?;
            // A value whose stored type does not match the definition (a presence left over
            // from an earlier definition) stays in the unknown list alone.
            (value.attr_type() == def.ty).then(|| ValueEntry::Defined {
                def: def.clone(),
                value: value.clone(),
            })
        })
        .collect();
    let unknown = model
        .unknown
        .get(&id_text)
        .into_iter()
        .flatten()
        .cloned()
        .map(ValueEntry::Unknown);
    defined.extend(unknown);
    defined.sort_by_key(|entry| entry.name_text().to_owned());
    Ok(defined)
}

impl ValueEntry {
    /// The attribute name of the entry.
    pub fn name_text(&self) -> &str {
        match self {
            Self::Defined { def, .. } => &def.name,
            Self::Unknown(unknown) => &unknown.name,
        }
    }
}

/// One schema or value change committed as the next revision, or replayed from the head state.
///
/// Steps, each failing closed: open and verify the head; read the attribute state; decide with
/// the engine; decide replay from the host ledger and the head state; author one change with
/// U02; check the layer engine's writes against the declared footprint; commit through U06/U07
/// with the strict verifier.
fn publish(
    store: &RevisionStore,
    context: &AttrContext,
    command_fingerprint: ContentDigest,
    same_state: impl Fn(&AttrModel) -> bool,
    change: impl FnOnce(&AttrModel) -> Result<(Footprint, Change), PlateError>,
    hook: &mut dyn CommitHook,
) -> Result<AttrChange, PlateError> {
    let control = control();
    let (head, base) = load_base(store, &context.project, &context.workspace)?;
    let before: BTreeMap<String, _> = base
        .assets
        .iter()
        .map(|a| (a.name.clone(), a.digest))
        .collect();
    let base_request = open_base(&base)?;
    let model = attr_model_of(&base_request, &control)?;
    let Base {
        project,
        workspace,
        generation: base_generation,
        expected,
        ..
    } = base;
    let op_host = host_uuid(&context.op_id.to_string())?;
    let (prior, recorded) = prior_op_with(store, &op_host, || {
        if same_state(&model) {
            Ok(command_fingerprint)
        } else {
            digest_of(b"no accepted request matches the head state")
        }
    })?;
    match prior {
        PriorOp::Accepted { fingerprint: seen } if seen == command_fingerprint => {
            return Ok(AttrChange {
                replayed: true,
                revision: replay_summary(&head, recorded)?,
                footprint: None,
                changed_layers: Vec::new(),
            });
        }
        PriorOp::Accepted { .. } => {
            return Err(CommandError::new(
                ErrorCode::OpIdReuse,
                "op_id",
                "operation id was accepted with different content",
            )
            .into());
        }
        PriorOp::Unseen => {}
    }
    // The engine's own decision gives every no-op its honest refusal (a duplicate define,
    // a missing definition, a stored value, an absent value); the head-state check above
    // only turns an accepted operation's retry into a replay.
    let (footprint, change) = change(&model)?;
    let schema = schema_identity()?;
    let (mut author, opened) = Author::open(
        base_request,
        EditTargets {
            authored: AUTHORED_LAYER.into(),
            derived: DERIVED_LAYER.into(),
        },
        schema,
        &control,
    )?;
    let prepared = author.prepare(
        Prepare {
            context: opened.context.clone(),
            op_id: context.op_id.to_string(),
            actor: context.actor.to_string(),
            schema,
            resources: Vec::new(),
            changes: vec![change],
        },
        &control,
    )?;
    check_footprint(&prepared, &footprint)?;
    let changed = changed_layers(&prepared, &before);
    let committed = publish_revision(
        store,
        &PublishRequest {
            footprint: &footprint,
            fingerprint: command_fingerprint,
            prepared: &prepared,
            schema,
            project: &project,
            workspace: &workspace,
            revision_id: &context.revision_id,
            op: op_host,
            base_generation,
            expected: &expected,
        },
        hook,
    )?;
    if committed.outcome == CommitOutcome::Replayed {
        return Err(integrity("an unseen operation was reported as replayed"));
    }
    Ok(AttrChange {
        replayed: false,
        revision: summarize(&committed.info),
        footprint: Some(footprint),
        changed_layers: changed,
    })
}

/// Defines one attribute and publishes the next revision, or replays the operation.
pub fn define_attribute(
    store: &RevisionStore,
    context: &AttrContext,
    command: &DefineAttribute,
    hook: &mut dyn CommitHook,
) -> Result<AttrChange, PlateError> {
    let fingerprint = command.content_fingerprint()?;
    publish(
        store,
        context,
        fingerprint,
        |model| model.schema.get(&command.def.name) == Some(&command.def),
        |model| {
            let decided = model.schema.define(command)?;
            Ok((
                decided.footprint,
                Change::AttrDefine {
                    def: command.def.clone(),
                },
            ))
        },
        hook,
    )
}

/// Undefines one attribute, keeping every stored value, or replays the operation.
pub fn undefine_attribute(
    store: &RevisionStore,
    context: &AttrContext,
    command: &UndefineAttribute,
    hook: &mut dyn CommitHook,
) -> Result<AttrChange, PlateError> {
    let fingerprint = command.content_fingerprint()?;
    publish(
        store,
        context,
        fingerprint,
        |model| !model.schema.contains(&command.name),
        |model| {
            let decided = model.schema.undefine(command)?;
            Ok((
                decided.footprint,
                Change::AttrUndefine {
                    name: command.name.clone(),
                },
            ))
        },
        hook,
    )
}

/// Stores one value of one defined attribute on one element, or replays the operation.
pub fn set_attribute(
    store: &RevisionStore,
    context: &AttrContext,
    command: &SetAttribute,
    hook: &mut dyn CommitHook,
) -> Result<AttrChange, PlateError> {
    let fingerprint = command.content_fingerprint()?;
    publish(
        store,
        context,
        fingerprint,
        |model| model.values.value_of(&command.target_id, &command.name) == Some(&command.value),
        |model| {
            let decided = model.values.set(&model.schema, command)?;
            Ok((
                decided.footprint,
                Change::AttrSet {
                    id: command.target_id.to_string(),
                    name: command.name.clone(),
                    value: Some(command.value.clone()),
                },
            ))
        },
        hook,
    )
}

/// Removes one stored value of one element, or replays the operation.
pub fn clear_attribute(
    store: &RevisionStore,
    context: &AttrContext,
    command: &ClearAttribute,
    hook: &mut dyn CommitHook,
) -> Result<AttrChange, PlateError> {
    let fingerprint = command.content_fingerprint()?;
    publish(
        store,
        context,
        fingerprint,
        |model| {
            model
                .values
                .value_of(&command.target_id, &command.name)
                .is_none()
        },
        |model| {
            let decided = model.values.clear(command)?;
            Ok((
                decided.footprint,
                Change::AttrSet {
                    id: command.target_id.to_string(),
                    name: command.name.clone(),
                    value: None,
                },
            ))
        },
        hook,
    )
}

/// Parses a `--value` text under a definition's type.
pub fn parse_value(ty: AttrType, raw: &str) -> Result<AttrValue, PlateError> {
    let invalid = || PlateError {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message: "the value does not fit the attribute's type",
    };
    Ok(match ty {
        AttrType::Text => AttrValue::Text(raw.to_owned()),
        AttrType::Choice => AttrValue::Choice(raw.to_owned()),
        AttrType::Number => AttrValue::Number(raw.parse::<f64>().map_err(|_| invalid())?),
        AttrType::Integer => AttrValue::Integer(raw.parse::<i32>().map_err(|_| invalid())?),
        AttrType::Boolean => match raw {
            "true" => AttrValue::Boolean(true),
            "false" => AttrValue::Boolean(false),
            _ => return Err(invalid()),
        },
    })
}

/// The definition of one attribute name in the head, for the command layer.
pub fn definition_of(store: &RevisionStore, name: &str) -> Result<Option<AttrDef>, PlateError> {
    Ok(attr_model(store)?.schema.get(name).cloned())
}
