// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A14.3 adapter: `CreateBoltGroup`, committed as a revision, plus a read of the fastener groups
//! of the head.
//!
//! A fastener group is an element of kind `connection` whose one `bim:connects` record names 2
//! to 1024 distinct parts, the main part first, and whose `bim:fastener` relationship pins one
//! fastener definition (A14.1). The row is the bolt amendment's line: two points, a reference
//! plane with a rotation, `on_plane` and `depth` offsets, an offset start and the spacings.
//! When the command names a fastener library, the resolved entry is pinned in the same
//! transaction (one revision, one undo unit); otherwise the designation must already be pinned
//! in the head, as a member's profile is. The engine
//! ([`beamcraft_engine::bolt_group`]) admits the frame, the crossings and the length against
//! conservative part bounds; the layer engine re-checks the structural rules on the candidate
//! and the strict verifier checks the committed bytes.

use crate::assembly::read_prims;
use crate::bolt::world_bounds;
use crate::definition::DefinitionRecord;
use crate::plate::{
    AUTHORED_LAYER, DERIVED_LAYER, PlateError, RevisionSummary, base_of, check_footprint, control,
    decode, host_uuid, integrity, load_base, number, open_base, prior_op_with, property,
    replay_summary, summarize, text,
};
use crate::publication::{PublishRequest, publish_revision};
use beamcraft_engine::bolt::Bounds;
use beamcraft_engine::bolt_group::BoltGroupDecision;
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::transaction::PriorOp;
use beamcraft_engine::weld::{PartKind, Parts};
use beamcraft_host::revision::{CommitHook, CommitOutcome, RevisionError, RevisionStore, Uuid7};
use beamcraft_layer_engine::authored::{
    Author, BoltGroup as LayerBoltGroup, Change, EditTargets, Element, Placement as LayerPlacement,
    Prepare, Shape, schema_identity, validate_layers,
};
use beamcraft_layer_engine::{Control, OpenRequest, Reader, Value};
use beamcraft_model::command::{BoltGroupPlacement, BoltLengthMode, CommandError, CreateBoltGroup};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::fastener::HoleMode;
use beamcraft_model::id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId};
use std::collections::BTreeMap;

/// Identities of one A14.3 call beyond the command itself (the A05 context).
#[derive(Debug, Clone)]
pub struct BoltGroupContext {
    /// The logical operation.
    pub op_id: OpId,
    /// Identity of the revision this call creates.
    pub revision_id: RevisionId,
    /// The acting user (trusted context).
    pub actor: Uuid7,
    /// Project of a store that has no revision yet.
    pub project: ProjectId,
    /// Workspace of a store that has no revision yet.
    pub workspace: WorkspaceId,
}

/// One fastener group of the verified head.
#[derive(Debug, Clone, PartialEq)]
pub struct FastenerGroupView {
    /// Identity.
    pub id: ElementId,
    /// Label.
    pub name: String,
    /// The connected parts, main part first.
    pub connects: Vec<ElementId>,
    /// The pinned fastener definition.
    pub fastener: ContentDigest,
    /// The resolved shank length in millimetres.
    pub length_mm: f64,
    /// The stored placement and settings, as the group's own command record.
    pub command: CreateBoltGroup,
}

/// The head model the decisions read: element kinds, conservative bounds and the groups.
struct HeadModel {
    parts: Parts,
    bounds: BTreeMap<ElementId, Bounds>,
    groups: Vec<FastenerGroupView>,
}

/// Reads the verified model of one base: element kinds for the decisions, conservative world
/// bounds of the parts, and every fastener group.
fn model_of(request: &OpenRequest, control: &Control) -> Result<HeadModel, PlateError> {
    // The structural rules run before anything is read, as the A05 adapter does.
    validate_layers(request.clone(), schema_identity()?, control)?;
    let mut reader = Reader::default();
    let opened = reader.open(request.clone(), control)?;
    let prims = read_prims(&reader, &opened.context, control)?;
    let mut by_path = BTreeMap::new();
    for prim in &prims {
        if let Some(id) = prim
            .element_id
            .as_deref()
            .and_then(|i| ElementId::parse(i).ok())
        {
            by_path.insert(prim.path.as_str(), id);
        }
    }
    let mut model = HeadModel {
        parts: Parts::default(),
        bounds: BTreeMap::new(),
        groups: Vec::new(),
    };
    for prim in &prims {
        let Some(id) = by_path.get(prim.path.as_str()).copied() else {
            continue;
        };
        // Only owned 0.1.0 parametric elements are interpreted, as the structural validator does.
        let supported = !prim.sources.is_empty()
            && prim.sources.iter().all(|s| s.owned)
            && property(prim, "bim:schemaVersion").and_then(text) == Some("0.1.0")
            && property(prim, "bim:geometryMode").and_then(text) == Some("parametric");
        let kind = match property(prim, "bim:elementKind")
            .and_then(text)
            .filter(|_| supported)
        {
            Some("plate" | "beam" | "column") => PartKind::Part,
            Some("assembly") => PartKind::Assembly,
            Some("connection")
                if property(prim, "steel:connection:type").and_then(text)
                    != Some("fastener_group") =>
            {
                PartKind::Other
            }
            Some("connection") => {
                model.groups.push(group_of(prim, id, &by_path)?);
                PartKind::Other
            }
            _ => PartKind::Other,
        };
        model.parts.insert(id, kind);
    }
    let decoded = decode(&reader, &opened, control)?;
    model.bounds = world_bounds(&decoded)?;
    Ok(model)
}

/// The fastener group `prim` decodes to, or an integrity error: a committed layer is untrusted
/// input.
fn group_of(
    prim: &beamcraft_layer_engine::Prim,
    id: ElementId,
    by_path: &BTreeMap<&str, ElementId>,
) -> Result<FastenerGroupView, PlateError> {
    let damaged = || integrity("fastener group record is not readable");
    let number_of = |name| property(prim, name).and_then(number).ok_or_else(damaged);
    let vec3 = |name: &str| -> Result<[f64; 3], PlateError> {
        match property(prim, name) {
            Some(Value::Array(items)) => match items.as_slice() {
                [Value::Number(x), Value::Number(y), Value::Number(z)] => Ok([*x, *y, *z]),
                _ => Err(damaged()),
            },
            _ => Err(damaged()),
        }
    };
    let connects = {
        let targets = prim
            .relationships
            .iter()
            .find(|r| r.name == "bim:connects")
            .map(|r| r.targets.as_slice())
            .unwrap_or_default();
        targets
            .iter()
            .map(|path| by_path.get(path.as_str()).copied().ok_or_else(damaged))
            .collect::<Result<Vec<_>, _>>()?
    };
    let spacings = match property(prim, "steel:boltGroup:spacingsMm") {
        None => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| match v {
                Value::Number(n) => Ok(*n),
                _ => Err(damaged()),
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(damaged()),
    };
    let length_mode = property(prim, "steel:boltGroup:lengthMode")
        .and_then(text)
        .ok_or_else(damaged)?;
    let resolved_length = number_of("steel:boltGroup:lengthMm")?;
    let fixed_length_mm = match length_mode {
        "auto" => None,
        "fixed" => Some(number_of("steel:boltGroup:fixedLengthMm")?),
        _ => return Err(damaged()),
    };
    let optional_bool = |name: &str| -> Result<Option<bool>, PlateError> {
        match property(prim, name) {
            None => Ok(None),
            Some(Value::Bool(value)) => Ok(Some(*value)),
            _ => Err(damaged()),
        }
    };
    let hole_mode = match property(prim, "steel:boltGroup:holeMode").and_then(text) {
        Some("generate") => HoleMode::Generate,
        Some("none") => HoleMode::None,
        _ => return Err(damaged()),
    };
    let command = CreateBoltGroup {
        new_id: id,
        name: prim.display_name.clone().unwrap_or_default(),
        parts: connects.clone(),
        fastener: fastener_of(prim)?,
        placement: BoltGroupPlacement {
            point1_mm: vec3("steel:boltGroup:point1Mm")?,
            point2_mm: vec3("steel:boltGroup:point2Mm")?,
            plane_normal: vec3("steel:boltGroup:planeNormal")?,
            rotation_rad: number_of("steel:boltGroup:rotationRad")?,
            on_plane_mm: number_of("steel:boltGroup:onPlaneMm")?,
            depth_mm: number_of("steel:boltGroup:depthMm")?,
        },
        offset_start_mm: number_of("steel:boltGroup:offsetStartMm")?,
        spacings_mm: spacings,
        length_mode: BoltLengthMode::from_token(length_mode).ok_or_else(damaged)?,
        fixed_length_mm,
        washer_under_head: optional_bool("steel:boltGroup:washerUnderHead")?,
        washer_under_nut: optional_bool("steel:boltGroup:washerUnderNut")?,
        hole_mode,
        hole_clearance_mm: property(prim, "steel:boltGroup:holeClearanceMm").and_then(number),
    };
    Ok(FastenerGroupView {
        id,
        name: command.name.clone(),
        connects,
        fastener: command.fastener,
        length_mm: resolved_length,
        command,
    })
}

/// The pinned fastener digest of a group prim.
fn fastener_of(prim: &beamcraft_layer_engine::Prim) -> Result<ContentDigest, PlateError> {
    let targets = prim
        .relationships
        .iter()
        .find(|r| r.name == "bim:fastener")
        .map(|r| r.targets.as_slice())
        .unwrap_or_default();
    let [target] = targets else {
        return Err(integrity("fastener group record is not readable"));
    };
    target
        .rsplit('/')
        .next()
        .and_then(|name| name.strip_prefix("FST_"))
        .ok_or_else(|| integrity("fastener group record is not readable"))
        .and_then(|digest| {
            digest
                .parse()
                .map_err(|_| integrity("fastener group record is not readable"))
        })
}

/// The fastener groups of the verified head (empty for a store without a revision).
pub fn groups(store: &RevisionStore) -> Result<Vec<FastenerGroupView>, PlateError> {
    let control = control();
    let revision = match store.open() {
        Ok(revision) => revision,
        Err(RevisionError::NoRevision) => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let content = revision.manifest().content();
    let damaged = |_| integrity("head manifest identity is not a UUIDv7");
    let project = ProjectId::parse(content.project().as_str()).map_err(damaged)?;
    let workspace = WorkspaceId::parse(content.workspace().as_str()).map_err(damaged)?;
    let (_, base) = load_base(store, &project, &workspace)?;
    Ok(model_of(&open_base(&base)?, &control)?.groups)
}

/// The fastener group `id` as the operation committed at `sequence` created it: present in that
/// revision and absent (as any element) from the revision before it, with the request's
/// fingerprint. `None` when that revision did not create such a group, so the operation id
/// belongs to another request.
fn created_by(
    store: &RevisionStore,
    sequence: u64,
    id: &ElementId,
    fingerprint: &ContentDigest,
    control: &Control,
) -> Result<bool, PlateError> {
    let at = store.open_at(sequence)?;
    let model = model_of(&open_base(&base_of(&at)?)?, control)?;
    let Some(group) = model.groups.iter().find(|g| &g.id == id) else {
        return Ok(false);
    };
    if group.command.content_fingerprint().ok().as_ref() != Some(fingerprint) {
        return Ok(false);
    }
    if sequence > 1 {
        let before = store.open_at(sequence - 1)?;
        if model_of(&open_base(&base_of(&before)?)?, control)?
            .parts
            .kind(id)
            .is_some()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Whether the pinned model holds a fastener group of this identity (for `delete-element` and
/// `copy-element`).
pub(crate) fn is_group(
    request: &OpenRequest,
    id: &ElementId,
    control: &Control,
) -> Result<bool, PlateError> {
    Ok(model_of(request, control)?
        .groups
        .iter()
        .any(|g| &g.id == id))
}

/// Creates one fastener group and publishes the next revision, or replays the operation.
///
/// `record` is the resolved fastener definition; `pin` decides whether it is authored in this
/// transaction (the command named a library) or already lives in the head. One revision, one
/// undo unit.
pub fn create_bolt_group(
    store: &RevisionStore,
    context: &BoltGroupContext,
    command: &CreateBoltGroup,
    record: &DefinitionRecord,
    pin: bool,
    hook: &mut dyn CommitHook,
) -> Result<CreatedGroup, PlateError> {
    command.validate_arguments()?;
    let DefinitionRecord::Fastener { spec } = record else {
        return Err(PlateError {
            rule_id: None,
            code: ErrorCode::InvalidArgument,
            message: "the fastener designation is not a fastener definition",
        });
    };
    let control = control();
    let (head, base) = load_base(store, &context.project, &context.workspace)?;
    let before: BTreeMap<String, _> = base
        .assets
        .iter()
        .map(|a| (a.name.clone(), a.digest))
        .collect();
    let base_request = open_base(&base)?;
    let model = model_of(&base_request, &control)?;
    let op_host = host_uuid(&context.op_id.to_string())?;
    let (prior, recorded) = prior_op_with(store, &op_host, || {
        crate::plate::digest_of(b"the accepted request is decided from the operation's revision")
    })?;
    if let PriorOp::Accepted { .. } = prior {
        // The ledger proves the operation committed; what it committed is read from its own
        // revision, never from the head, so another group that happens to match is not a replay.
        let replay = match recorded {
            Some((_, sequence, _)) => created_by(
                store,
                sequence,
                &command.new_id,
                &command.content_fingerprint()?,
                &control,
            )?,
            None => false,
        };
        if replay {
            return Ok(CreatedGroup {
                replayed: true,
                revision: replay_summary(&head, recorded)?,
                footprint: None,
                changed_layers: Vec::new(),
                groups: model.groups,
            });
        }
        return Err(CommandError::new(
            ErrorCode::OpIdReuse,
            "op_id",
            "operation id was accepted with different content",
        )
        .into());
    }
    let decision: BoltGroupDecision =
        model
            .parts
            .create_bolt_group(command, spec, &model.bounds, pin)?;
    let resource = if pin { Some(record.resource()?) } else { None };
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
            resources: resource.into_iter().collect(),
            changes: vec![Change::Create(Element {
                id: command.new_id.to_string(),
                name: command.name.clone(),
                shape: Shape::BoltGroup(LayerBoltGroup {
                    connects: command.parts.iter().map(|p| p.to_string()).collect(),
                    fastener: command.fastener,
                    point1_mm: command.placement.point1_mm,
                    point2_mm: command.placement.point2_mm,
                    plane_normal: command.placement.plane_normal,
                    rotation_rad: command.placement.rotation_rad,
                    on_plane_mm: command.placement.on_plane_mm,
                    depth_mm: command.placement.depth_mm,
                    offset_start_mm: command.offset_start_mm,
                    spacings_mm: command.spacings_mm.clone(),
                    length_mode: command.length_mode.token().to_owned(),
                    length_mm: decision.length_mm,
                    fixed_length_mm: command.fixed_length_mm,
                    washer_under_head: command.washer_under_head,
                    washer_under_nut: command.washer_under_nut,
                    hole_mode: command.hole_mode.token().to_owned(),
                    hole_clearance_mm: command.hole_clearance_mm,
                }),
                placement: LayerPlacement {
                    translation_mm: [0.0; 3],
                    orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
                },
                material: None,
            })],
        },
        &control,
    )?;
    check_footprint(&prepared, &decision.footprint)?;
    let changed_layers = prepared
        .layers
        .iter()
        .filter(|l| before.get(&l.name) != Some(&l.digest))
        .map(|l| l.name.clone())
        .collect();
    let committed = publish_revision(
        store,
        &PublishRequest {
            footprint: &decision.footprint,
            fingerprint: command.content_fingerprint()?,
            prepared: &prepared,
            schema,
            project: &base.project,
            workspace: &base.workspace,
            revision_id: &context.revision_id,
            op: op_host,
            base_generation: base.generation,
            expected: &base.expected,
        },
        hook,
    )?;
    Ok(CreatedGroup {
        replayed: committed.outcome == CommitOutcome::Replayed,
        revision: summarize(&committed.info),
        footprint: Some(decision.footprint),
        changed_layers,
        groups: model.groups,
    })
}

/// Result of `create_bolt_group`.
#[derive(Debug, Clone)]
pub struct CreatedGroup {
    /// True when the operation was already committed: nothing was written.
    pub replayed: bool,
    /// The head revision after the call.
    pub revision: RevisionSummary,
    /// The declared footprint of the create, `None` on a replay.
    pub footprint: Option<Footprint>,
    /// The layers the call changed.
    pub changed_layers: Vec<String>,
    /// The fastener groups of the head before the call.
    pub groups: Vec<FastenerGroupView>,
}
