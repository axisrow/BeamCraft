// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A05 adapter: `CreateBolt`, committed as the next revision, plus a read of the bolt groups of
//! the head.
//!
//! A bolt group is an element of kind `connection` with its own UUIDv7, label and identity
//! placement, explicitly authored bolt properties (diameter, count, pitch, world origin, unit
//! world direction) and one `bim:connects` record naming the main part, then the part bolted to
//! it. Like an A06 weld it has no material and no derived body; unlike a weld it carries a world
//! placement of its bolts, and the engine refuses a group whose bolts do not lie on both parts
//! (conservative bounds, see the A05 design note). The record lives with the group, so the parts
//! are only read. The engine ([`beamcraft_engine::bolt`]) decides the identity, the parts' kinds
//! and the reachability from the head model; the layer engine re-checks the structural rules on
//! the candidate and the strict verifier checks the committed bytes.
//!
//! Replay of an `op_id` is decided from the revision the host ledger recorded for it: the
//! operation is a replay only when that revision created exactly this group (present there,
//! absent from the revision before), whatever the head holds now. No command renames, edits or
//! moves a group yet; its element placement is the identity. Deleting a bolted part is
//! `DEPENDENCY_CONFLICT` while the group connects it (no cascade); deleting the group is
//! `delete-element`. Copying a group is refused (`DV-REL-011`). A stored `connection` with weld
//! properties is never decoded as a bolt group and vice versa.

use crate::assembly::read_prims;
use crate::definition::profile_lookup;
use crate::plate::{
    AUTHORED_LAYER, DERIVED_LAYER, PlateError, RevisionSummary, base_of, check_footprint, control,
    decode, digest_of, host_uuid, integrity, load_base, number, open_base, prior_op_with, property,
    replay_summary, summarize, text,
};
use crate::publication::{PublishRequest, publish_revision};
use beamcraft_engine::bolt::Bounds;
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::transaction::PriorOp;
use beamcraft_engine::weld::{PartKind, Parts};
use beamcraft_host::revision::{CommitHook, CommitOutcome, RevisionError, RevisionStore, Uuid7};
use beamcraft_layer_engine::authored::{
    Author, Bolt as LayerBolt, Change, EditTargets, Element, Placement as LayerPlacement, Prepare,
    Shape, schema_identity, validate_layers,
};
use beamcraft_layer_engine::{Control, OpenRequest, Reader, Value};
use beamcraft_model::command::{BoltSpec, CommandError, CreateBolt};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId};
use std::collections::BTreeMap;

/// Identities of one A05 call beyond the command itself.
#[derive(Debug, Clone)]
pub struct BoltContext {
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

/// One bolt group of the head.
#[derive(Debug, Clone, PartialEq)]
pub struct BoltView {
    /// Identity.
    pub id: ElementId,
    /// Label.
    pub name: String,
    /// The main part.
    pub main: ElementId,
    /// The part bolted to it.
    pub secondary: ElementId,
    /// The authored properties.
    pub spec: BoltSpec,
}

/// Result of `create_bolt`.
#[derive(Debug, Clone, PartialEq)]
pub struct BoltChange {
    /// True when the same operation was already committed and nothing was written.
    pub replayed: bool,
    /// The revision that holds the result.
    pub revision: RevisionSummary,
    /// The declared footprint; `None` for a replay.
    pub footprint: Option<Footprint>,
    /// Names of the layers whose bytes changed (empty for a replay).
    pub changed_layers: Vec<String>,
    /// The bolt groups of the resulting head.
    pub bolts: Vec<BoltView>,
}

/// The kinds of a pinned model's elements, its bolt groups and the conservative world bounds of
/// its parts.
struct Model {
    parts: Parts,
    bolts: Vec<BoltView>,
    bounds: BTreeMap<ElementId, Bounds>,
}

fn bolt_of(
    prim: &beamcraft_layer_engine::Prim,
    id: ElementId,
    by_path: &BTreeMap<&str, ElementId>,
) -> Result<BoltView, PlateError> {
    let damaged = || integrity("bolt record is not readable");
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
    let count = match property(prim, "steel:bolt:count") {
        Some(Value::Int(n)) if (1..=512).contains(n) => *n as usize,
        _ => return Err(damaged()),
    };
    let spec = BoltSpec {
        diameter_mm: number_of("steel:bolt:diameterMm")?,
        count,
        pitch_mm: number_of("steel:bolt:pitchMm")?,
        origin_mm: vec3("steel:bolt:originMm")?,
        direction_mm: vec3("steel:bolt:directionMm")?,
    };
    let targets = prim
        .relationships
        .iter()
        .find(|r| r.name == "bim:connects")
        .map(|r| r.targets.as_slice())
        .unwrap_or_default();
    let [main, secondary] = targets else {
        return Err(damaged());
    };
    let part = |path: &String| by_path.get(path.as_str()).copied().ok_or_else(damaged);
    Ok(BoltView {
        id,
        name: property(prim, "bim:name")
            .and_then(text)
            .ok_or_else(damaged)?
            .to_owned(),
        main: part(main)?,
        secondary: part(secondary)?,
        spec,
    })
}

/// Conservative world bounds of every decoded part: a plate contributes the exact box of its
/// contour extruded by its thickness, a member its two world axis points expanded by half the
/// section's bounding-box diagonal in every axis (which contains it under any roll). A part with
/// a non-identity placement rotation is refused as unsupported rather than bounded wrongly.
fn world_bounds(
    decoded: &crate::plate::Decoded,
) -> Result<BTreeMap<ElementId, Bounds>, PlateError> {
    let unsupported = || {
        PlateError::from(CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "placement.orientation_xyzw",
            "rotated part placement is not supported for bolts in this slice",
        ))
    };
    let identity = |q: [f64; 4]| q[0] == 0.0 && q[1] == 0.0 && q[2] == 0.0;
    let mut bounds = BTreeMap::new();
    let profiles = profile_lookup(&decoded.definitions);
    for (authored, _) in &decoded.plates {
        let translation = authored.placement.translation_mm;
        if !identity(authored.placement.orientation_xyzw) {
            return Err(unsupported());
        }
        let Some(first) = authored.contour_points_mm.first() else {
            return Err(integrity("a committed plate has no contour points"));
        };
        let mut min = *first;
        let mut max = *first;
        for p in &authored.contour_points_mm {
            min[0] = min[0].min(p[0]);
            min[1] = min[1].min(p[1]);
            min[2] = min[2].min(p[2]);
            max[0] = max[0].max(p[0]);
            max[1] = max[1].max(p[1]);
            max[2] = max[2].max(p[2]);
        }
        max[2] += authored.thickness_mm;
        let world =
            |[x, y, z]: [f64; 3]| [x + translation[0], y + translation[1], z + translation[2]];
        bounds.insert(authored.id, [world(min), world(max)]);
    }
    for (authored, _) in &decoded.members {
        let translation = authored.placement.translation_mm;
        if !identity(authored.placement.orientation_xyzw) {
            return Err(unsupported());
        }
        let (width, height) = profiles(&authored.profile)?.extent_mm();
        // Half the diagonal of the section's bounding box contains every rolled cross-section.
        // The box spans the whole path (one chord for a straight member, the polyline for a
        // polybeam) plus that reach.
        let reach = 0.5 * (width * width + height * height).sqrt();
        let Some(first) = authored.axis_points_mm.first() else {
            return Err(integrity("a committed member has no path points"));
        };
        let mut corner = *first;
        let mut opposite = *first;
        for p in &authored.axis_points_mm {
            corner[0] = corner[0].min(p[0]);
            corner[1] = corner[1].min(p[1]);
            corner[2] = corner[2].min(p[2]);
            opposite[0] = opposite[0].max(p[0]);
            opposite[1] = opposite[1].max(p[1]);
            opposite[2] = opposite[2].max(p[2]);
        }
        let expand =
            |[x, y, z]: [f64; 3], sign: f64| [x + sign * reach, y + sign * reach, z + sign * reach];
        let min = expand(
            [
                corner[0] + translation[0],
                corner[1] + translation[1],
                corner[2] + translation[2],
            ],
            -1.0,
        );
        let max = expand(
            [
                opposite[0] + translation[0],
                opposite[1] + translation[1],
                opposite[2] + translation[2],
            ],
            1.0,
        );
        bounds.insert(authored.id, [min, max]);
    }
    Ok(bounds)
}

fn model_of(request: &OpenRequest, control: &Control) -> Result<Model, PlateError> {
    // The structural rules (A05-V01 and A06-V01 among them) run before anything is read, so a
    // damaged record is refused rather than reported or replayed.
    validate_layers(request.clone(), schema_identity()?, control)?;
    let mut reader = Reader::default();
    let opened = reader.open(request.clone(), control)?;
    let prims = read_prims(&reader, &opened.context, control)?;
    let by_path: BTreeMap<&str, ElementId> = prims
        .iter()
        .filter_map(|p| {
            let id = ElementId::parse(p.element_id.as_deref()?).ok()?;
            Some((p.path.as_str(), id))
        })
        .collect();
    let mut model = Model {
        parts: Parts::default(),
        bolts: Vec::new(),
        bounds: BTreeMap::new(),
    };
    for prim in &prims {
        let Some(id) = by_path.get(prim.path.as_str()).copied() else {
            continue;
        };
        // Only owned 0.1.0 parametric elements are interpreted, as the structural validator does;
        // newer, foreign or other-mode content stays inert and is never decoded as a bolt group.
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
                if property(prim, "steel:connection:type").and_then(text) != Some("bolt_group") =>
            {
                PartKind::Other
            }
            Some("connection") => {
                model.bolts.push(bolt_of(prim, id, &by_path)?);
                PartKind::Other
            }
            _ => PartKind::Other,
        };
        model.parts.insert(id, kind);
    }
    model.bolts.sort_by_key(|b| b.id);
    let decoded = decode(&reader, &opened, control)?;
    model.bounds = world_bounds(&decoded)?;
    Ok(model)
}

/// The bolt group `id` as the operation committed at `sequence` created it: present in that
/// revision and absent (as any element) from the revision before it. `None` when that revision
/// did not create such a group, so the operation id belongs to another request.
fn created_by(
    store: &RevisionStore,
    sequence: u64,
    id: &ElementId,
    control: &Control,
) -> Result<Option<BoltView>, PlateError> {
    let at = store.open_at(sequence)?;
    let model = model_of(&open_base(&base_of(&at)?)?, control)?;
    let Some(bolt) = model.bolts.into_iter().find(|b| b.id == *id) else {
        return Ok(None);
    };
    if sequence > 1 {
        let before = store.open_at(sequence - 1)?;
        if model_of(&open_base(&base_of(&before)?)?, control)?
            .parts
            .kind(id)
            .is_some()
        {
            return Ok(None);
        }
    }
    Ok(Some(bolt))
}

/// Whether the pinned model holds a bolt group of this identity (for `delete-element` and
/// `copy-element`).
pub(crate) fn is_bolt(
    request: &OpenRequest,
    id: &ElementId,
    control: &Control,
) -> Result<bool, PlateError> {
    Ok(model_of(request, control)?
        .bolts
        .iter()
        .any(|b| b.id == *id))
}

/// The bolt groups of the verified head (empty for a store without a revision).
pub fn bolts(store: &RevisionStore) -> Result<Vec<BoltView>, PlateError> {
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
    Ok(model_of(&open_base(&base)?, &control)?.bolts)
}

/// Creates one bolt group between two existing parts and publishes the next revision, or replays
/// the operation.
///
/// Steps, each failing closed: open and verify the head; decide replay from the host ledger and
/// the head state; decide the identity, the parts' kinds and the reachability with the engine;
/// author with U02; check the layer engine's writes against the declared footprint; commit
/// through U06/U07 with the strict verifier.
pub fn create_bolt(
    store: &RevisionStore,
    context: &BoltContext,
    command: &CreateBolt,
    hook: &mut dyn CommitHook,
) -> Result<BoltChange, PlateError> {
    command.validate_arguments()?;
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
    // The ledger proves that the operation committed; what it committed is read from its own
    // revision, never from the head, so another group that happens to match is not a replay.
    let (prior, recorded) = prior_op_with(store, &op_host, || {
        digest_of(b"the accepted request is decided from the operation's revision")
    })?;
    let accepted = match (&prior, &recorded) {
        (PriorOp::Accepted { .. }, Some((_, sequence, _))) => {
            created_by(store, *sequence, &command.new_id, &control)?
        }
        _ => None,
    };
    let same = accepted.as_ref().is_some_and(|b| {
        b.name == command.name
            && b.main == command.main_id
            && b.secondary == command.secondary_id
            && b.spec == command.bolt
    });
    match prior {
        PriorOp::Accepted { .. } if same => {
            return Ok(BoltChange {
                replayed: true,
                revision: replay_summary(&head, recorded)?,
                footprint: None,
                changed_layers: Vec::new(),
                bolts: model.bolts,
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
    let footprint = model.parts.create_bolt(command, &model.bounds)?;
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
            changes: vec![Change::Create(Element {
                id: command.new_id.to_string(),
                name: command.name.clone(),
                shape: Shape::Bolt(LayerBolt {
                    connects: [
                        command.main_id.to_string(),
                        command.secondary_id.to_string(),
                    ],
                    diameter_mm: command.bolt.diameter_mm,
                    count: command.bolt.count,
                    pitch_mm: command.bolt.pitch_mm,
                    origin_mm: command.bolt.origin_mm,
                    direction_mm: command.bolt.direction_mm,
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
    check_footprint(&prepared, &footprint)?;
    let changed_layers = prepared
        .layers
        .iter()
        .filter(|l| before.get(&l.name) != Some(&l.digest))
        .map(|l| l.name.clone())
        .collect();
    let committed = publish_revision(
        store,
        &PublishRequest {
            footprint: &footprint,
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
    if committed.outcome == CommitOutcome::Replayed {
        return Err(integrity("an unseen operation was reported as replayed"));
    }
    Ok(BoltChange {
        replayed: false,
        revision: summarize(&committed.info),
        footprint: Some(footprint),
        changed_layers,
        bolts: bolts(store)?,
    })
}
