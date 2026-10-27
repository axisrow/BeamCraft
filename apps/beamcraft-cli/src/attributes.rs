// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! I04 transactions and pinned user-property reads, independent of geometry evaluation.
use crate::{
    plate::{self, AUTHORED_LAYER, DERIVED_LAYER, PlateError, RevisionSummary},
    publication::{PublishRequest, publish_revision},
};
use beamcraft_host::{
    ledger::Outcome,
    revision::{CommitHook, Revision, RevisionStore, Uuid7},
};
use beamcraft_layer_engine::{
    Prim, Property, Reader, Value,
    authored::{
        self, Author, Change, EditTargets, Element, Placement, Prepare, Shape, schema_identity,
    },
};
use beamcraft_model::{
    attribute::{AttributeDefinition, AttributeValue, MAX_VALUES},
    error::ErrorCode,
    id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId},
};
use std::collections::BTreeMap;
/// Facts for one owned element; unknown user properties retain their complete typed payload.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementAttributes {
    /// Element kind token.
    pub kind: String,
    /// Readable name.
    pub name: String,
    /// Optional authored class; only parts use it.
    pub class: Option<i32>,
    /// Hidden/inactive elements remain inspectable but are not display targets.
    pub deactivated: bool,
    /// Admitted owned schema and mode; future/foreign targets are read-only.
    pub editable: bool,
    /// User property names, native types and exact values, including unknown data.
    pub properties: Vec<Property>,
}
/// Pinned immutable user-property snapshot for API, CLI and filters.
#[derive(Debug, Clone, PartialEq)]
pub struct Attributes {
    /// The exact model revision read.
    pub revision: Option<RevisionSummary>,
    /// Immutable typed registry.
    pub definitions: BTreeMap<ElementId, AttributeDefinition>,
    /// Per-element metadata and user properties.
    pub elements: BTreeMap<ElementId, ElementAttributes>,
}
/// Common mutation identities.
#[derive(Debug, Clone)]
pub struct AttributeContext {
    /// Logical operation UUID.
    pub op_id: OpId,
    /// New revision UUID.
    pub revision_id: RevisionId,
    /// Actor label.
    pub actor: Uuid7,
    /// Fallback project.
    pub project: ProjectId,
    /// Fallback workspace.
    pub workspace: WorkspaceId,
}
/// Immutable definition creation or one element's explicit scalar patch.
#[derive(Debug, Clone)]
pub enum AttributeCommand {
    /// Create one new immutable definition.
    Define(AttributeDefinition),
    /// Set or remove explicitly addressed scalar properties.
    Patch {
        /// Target element UUID.
        id: ElementId,
        /// None explicitly removes a property.
        values: BTreeMap<ElementId, Option<AttributeValue>>,
    },
}
/// One admitted publication result.
#[derive(Debug, Clone)]
pub struct Changed {
    /// An accepted request was replayed.
    pub replayed: bool,
    /// The committed revision.
    pub revision: RevisionSummary,
}
fn invalid(message: &'static str) -> PlateError {
    PlateError::new(ErrorCode::InvalidArgument, message)
}
pub(crate) fn of_prims(
    prims: &[Prim],
    revision: Option<RevisionSummary>,
) -> Result<Attributes, PlateError> {
    let definitions = authored::attribute::definitions(prims)?;
    beamcraft_engine::attribute::validate(&definitions)?;
    let mut elements = BTreeMap::new();
    for p in prims {
        if p.sources.is_empty() || p.sources.iter().any(|s| !s.owned) {
            continue;
        }
        let Some(id) = p
            .element_id
            .as_deref()
            .and_then(|s| ElementId::parse(s).ok())
        else {
            continue;
        };
        let kind = plate::property(p, "bim:elementKind")
            .and_then(plate::text)
            .unwrap_or("unknown")
            .to_owned();
        let editable = !p.sources.is_empty()
            && p.sources.iter().all(|s| s.owned)
            && plate::property(p, "bim:schemaVersion").and_then(plate::text) == Some("0.1.0")
            && matches!(
                plate::property(p, "bim:geometryMode").and_then(plate::text),
                Some("parametric" | "reference")
            );
        let class = match plate::property(p, "steel:class") {
            Some(Value::Int(v)) => i32::try_from(*v).ok(),
            _ if matches!(kind.as_str(), "plate" | "beam" | "column") => Some(0),
            _ => None,
        };
        let properties = p
            .properties
            .iter()
            .filter(|v| v.name.starts_with("user:"))
            .take(MAX_VALUES + 1)
            .cloned()
            .collect::<Vec<_>>();
        if properties.len() > MAX_VALUES {
            return Err(invalid("element user-property limit exceeded"));
        }
        elements.insert(
            id,
            ElementAttributes {
                kind,
                name: plate::property(p, "bim:name")
                    .and_then(plate::text)
                    .unwrap_or("")
                    .to_owned(),
                class,
                deactivated: !p.active
                    || plate::property(p, "bim:isDeactivated") == Some(&Value::Bool(true)),
                editable,
                properties,
            },
        );
    }
    Ok(Attributes {
        revision,
        definitions,
        elements,
    })
}
fn read_request(
    request: beamcraft_layer_engine::OpenRequest,
    revision: Option<RevisionSummary>,
) -> Result<Attributes, PlateError> {
    let control = plate::control();
    authored::validate_layers(request.clone(), schema_identity()?, &control)?;
    let mut reader = Reader::default();
    let opened = reader.open(request, &control)?;
    of_prims(
        &crate::assembly::read_prims(&reader, &opened.context, &control)?,
        revision,
    )
}
/// Read an exact verified revision, including unknown user properties.
pub fn read_revision(revision: &Revision) -> Result<Attributes, PlateError> {
    read_request(
        plate::open_base(&plate::base_of(revision)?)?,
        Some(plate::summarize(revision.info())),
    )
}
/// Read the current model, or an empty registry for a fresh store.
pub fn read(store: &RevisionStore) -> Result<Attributes, PlateError> {
    match store.open() {
        Ok(r) => read_revision(&r),
        Err(beamcraft_host::revision::RevisionError::NoRevision) => Ok(Attributes {
            revision: None,
            definitions: BTreeMap::new(),
            elements: BTreeMap::new(),
        }),
        Err(e) => Err(e.into()),
    }
}
/// Resolve an exact definition UUID or unique case-sensitive name.
pub fn definition<'a>(
    attrs: &'a Attributes,
    selector: &str,
) -> Result<&'a AttributeDefinition, PlateError> {
    if let Ok(id) = ElementId::parse(selector) {
        return attrs
            .definitions
            .get(&id)
            .ok_or_else(|| PlateError::new(ErrorCode::NotFound, "attribute definition not found"));
    }
    attrs
        .definitions
        .values()
        .find(|d| d.name == selector)
        .ok_or_else(|| PlateError::new(ErrorCode::NotFound, "attribute definition not found"))
}
/// Apply one complete command as one revision/undo unit, with immutable request replay evidence.
pub fn apply(
    store: &RevisionStore,
    context: &AttributeContext,
    command: &AttributeCommand,
    hook: &mut dyn CommitHook,
) -> Result<Changed, PlateError> {
    let fingerprint = match command {
        AttributeCommand::Define(d) => d.fingerprint()?,
        AttributeCommand::Patch { id, values } => {
            beamcraft_model::attribute::patch_fingerprint(*id, values)?
        }
    };
    let op = plate::host_uuid(&context.op_id.to_string())?;
    match store.query_outcome(&op)? {
        Outcome::Committed { sequence, .. } => {
            let revision = store.open_at(sequence)?;
            let capture = crate::command_evidence::read(&revision)?.ok_or_else(|| {
                PlateError::new(ErrorCode::OpIdReuse, "operation has incompatible evidence")
            })?;
            if capture.fingerprint != fingerprint {
                return Err(PlateError::new(
                    ErrorCode::OpIdReuse,
                    "operation was accepted with different attribute arguments",
                ));
            }
            return Ok(Changed {
                replayed: true,
                revision: plate::summarize(revision.info()),
            });
        }
        Outcome::Unknown => {
            return Err(PlateError::new(
                ErrorCode::OutcomeUnknown,
                "attribute operation requires outcome recovery",
            ));
        }
        Outcome::Aborted { .. } | Outcome::NotFound => {}
    }
    let (head, base) = plate::load_base(store, &context.project, &context.workspace)?;
    let request = plate::open_base(&base)?;
    let attrs = read_request(
        request.clone(),
        head.as_ref().map(|r| plate::summarize(r.info())),
    )?;
    let (changes, footprint) = match command {
        AttributeCommand::Define(d) => {
            d.validate()?;
            if attrs.elements.contains_key(&d.id) {
                return Err(PlateError::new(
                    ErrorCode::DuplicateId,
                    "attribute identity is already used",
                ));
            }
            if attrs.definitions.len() >= beamcraft_model::attribute::MAX_DEFINITIONS {
                return Err(PlateError::new(
                    ErrorCode::LimitExceeded,
                    "too many attribute definitions",
                ));
            }
            if attrs.definitions.values().any(|old| old.name == d.name) {
                return Err(invalid("attribute name already exists"));
            }
            (
                vec![Change::Create(Element {
                    id: d.id.to_string(),
                    name: d.name.clone(),
                    shape: Shape::AttributeDefinition(d.clone()),
                    placement: Placement {
                        translation_mm: [0.; 3],
                        orientation_xyzw: [0., 0., 0., 1.],
                    },
                    material: None,
                })],
                beamcraft_engine::footprint::Footprint::for_create(d.id, None)?,
            )
        }
        AttributeCommand::Patch { id, values } => {
            let target = attrs.elements.get(id).ok_or_else(|| {
                PlateError::new(ErrorCode::NotFound, "attribute target not found")
            })?;
            if !target.editable {
                return Err(PlateError::new(
                    ErrorCode::ForeignReadOnly,
                    "attribute target is foreign or unsupported",
                ));
            }
            let mut names = target
                .properties
                .iter()
                .map(|p| p.name.clone())
                .collect::<std::collections::BTreeSet<_>>();
            let mut changed = false;
            for (key, value) in values {
                let name = beamcraft_model::attribute::property_name(*key);
                let held = target.properties.iter().find(|p| p.name == name);
                match value {
                    Some(v) => {
                        changed |= held.and_then(authored::attribute::scalar).as_ref() != Some(v);
                        names.insert(name);
                    }
                    None => {
                        changed |= names.remove(&name);
                    }
                }
            }
            if !changed {
                return Err(invalid("attribute patch changes nothing"));
            }
            if names.len() > MAX_VALUES {
                return Err(PlateError::new(
                    ErrorCode::LimitExceeded,
                    "too many user properties on one element",
                ));
            }
            (
                vec![Change::UserAttributes {
                    id: id.to_string(),
                    values: values.clone(),
                }],
                beamcraft_engine::attribute::patch(&attrs.definitions, *id, values)?,
            )
        }
    };
    let control = plate::control();
    let schema = schema_identity()?;
    let (mut author, opened) = Author::open(
        request,
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
            changes,
        },
        &control,
    )?;
    plate::check_footprint(&prepared, &footprint)?;
    let committed = publish_revision(
        store,
        &PublishRequest {
            footprint: &footprint,
            fingerprint,
            prepared: &prepared,
            schema,
            project: &base.project,
            workspace: &base.workspace,
            revision_id: &context.revision_id,
            op,
            base_generation: base.generation,
            expected: &base.expected,
        },
        hook,
    )?;
    Ok(Changed {
        replayed: false,
        revision: plate::summarize(&committed.info),
    })
}

/// Carry typed user properties into a part copy; unknown/opaque data refuses instead of vanishing.
pub(crate) fn copy_properties(
    attrs: &Attributes,
    source: ElementId,
    target: ElementId,
) -> Result<(Vec<Change>, beamcraft_engine::footprint::Footprint), PlateError> {
    let Some(element) = attrs.elements.get(&source) else {
        return Ok((Vec::new(), Default::default()));
    };
    let mut values = BTreeMap::new();
    for p in &element.properties {
        let id = authored::attribute::property_id(&p.name).ok_or_else(|| {
            PlateError::new(
                ErrorCode::DependencyUnavailable,
                "copy contains an unknown user property",
            )
        })?;
        let value = authored::attribute::scalar(p).ok_or_else(|| {
            PlateError::new(
                ErrorCode::DependencyUnavailable,
                "copy contains an opaque or blocked user property",
            )
        })?;
        if !attrs.definitions.contains_key(&id) {
            return Err(PlateError::new(
                ErrorCode::DependencyUnavailable,
                "copy has an unresolved user-property definition",
            ));
        }
        values.insert(id, Some(value));
    }
    if values.is_empty() {
        return Ok((Vec::new(), Default::default()));
    }
    let footprint = beamcraft_engine::attribute::patch(&attrs.definitions, target, &values)?;
    Ok((
        vec![Change::UserAttributes {
            id: target.to_string(),
            values,
        }],
        footprint,
    ))
}
