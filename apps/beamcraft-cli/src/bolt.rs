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
use crate::definition::{material_lookup, profile_lookup};
use crate::plate::{
    AUTHORED_LAYER, DERIVED_LAYER, PlateError, RevisionSummary, base_of, check_footprint, control,
    decode, digest_of, evaluate_decoded, host_uuid, integrity, load_base, mesh_change, number,
    open_base, plate_record, prior_op_with, property, replay_summary, summarize, text,
};
use crate::publication::{PublishRequest, changed_layers, publish_revision};
use beamcraft_engine::bolt::Bounds;
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::query::committed::PlateRecord;
use beamcraft_engine::transaction::PriorOp;
use beamcraft_engine::weld::{PartKind, Parts};
use beamcraft_host::revision::{CommitHook, CommitOutcome, RevisionError, RevisionStore, Uuid7};
use beamcraft_layer_engine::authored::{
    Author, Bolt as LayerBolt, BoltGroup as LayerBoltGroup, Change, EditTargets, Element,
    Placement as LayerPlacement, Prepare, Resource, Shape, schema_identity, validate_layers,
};
use beamcraft_layer_engine::{Control, OpenRequest, Reader, Value};
use beamcraft_model::command::{
    BoltGroupInputs, BoltLength, BoltSpacing, BoltSpec, CommandError, CreateBolt, CreateBoltGroup,
    HoleKind, HoleRecord, MAX_HOLES, UpdateBoltGroup,
};
use beamcraft_model::digest::ContentDigest;
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

/// The kinds of a pinned model's elements, its bolt groups of both forms, the pinned
/// definitions and the geometry its parts offer the placement and hole decisions.
struct Model {
    parts: Parts,
    bolts: Vec<BoltView>,
    groups: Vec<BoltGroupView>,
    definitions: BTreeMap<ContentDigest, crate::definition::DefinitionRecord>,
    bounds: BTreeMap<ElementId, Bounds>,
    /// World translation of every part (identity rotation enforced): the frame a generated
    /// hole's polygon is mapped into (A14.4).
    translations: BTreeMap<ElementId, [f64; 3]>,
    /// Identities of the decoded beams and columns: a generating group refuses them until G14.
    members: std::collections::BTreeSet<ElementId>,
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

/// The stored record of an A14.3 bolt group (bolt amendment section 4), decoded from the
/// authored properties. The engine's decisions (crossing, grip, length) are already in it.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupSpec {
    /// Content digest of the pinned fastener definition.
    pub fastener: ContentDigest,
    /// World start of the row in millimetres.
    pub point1_mm: [f64; 3],
    /// World end of the row.
    pub point2_mm: [f64; 3],
    /// Unit normal of the reference plane.
    pub plane_normal_mm: [f64; 3],
    /// Rotation about the row in radians.
    pub rotation_rad: f64,
    /// Offset of the whole group along the plane axis in millimetres.
    pub on_plane_mm: f64,
    /// Offset of the whole group along the shank axis in millimetres.
    pub depth_mm: f64,
    /// Distance from `point1` to the first fastener in millimetres.
    pub offset_start_mm: f64,
    /// `spacings`, `pitch` or `offset_end`.
    pub spacing_mode: String,
    /// The value of a generator; `0.0` for an explicit list.
    pub spacing_value_mm: f64,
    /// The authoritative distances between neighbouring fasteners.
    pub spacings_mm: Vec<f64>,
    /// `auto` or `fixed`.
    pub length_mode: String,
    /// The resolved shank length in millimetres.
    pub length_mm: f64,
    /// Whether a washer sits under the head.
    pub washer_under_head: bool,
    /// Whether a washer sits under the nut.
    pub washer_under_nut: bool,
    /// `generate` or `none` (bolt amendment section 5).
    pub hole_mode: String,
    /// The group's own hole clearance; `None` reads as the pinned definition's.
    pub hole_clearance_mm: Option<f64>,
}

/// One A14.3 bolt group of the head.
#[derive(Debug, Clone, PartialEq)]
pub struct BoltGroupView {
    /// Identity.
    pub id: ElementId,
    /// Label.
    pub name: String,
    /// Connected parts, main part first.
    pub parts: Vec<ElementId>,
    /// The stored record.
    pub spec: GroupSpec,
}

fn bolt_group_of(
    prim: &beamcraft_layer_engine::Prim,
    id: ElementId,
    by_path: &BTreeMap<&str, ElementId>,
) -> Result<BoltGroupView, PlateError> {
    let damaged = || integrity("bolt group record is not readable");
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
    let flag = |name: &str| match property(prim, name) {
        Some(Value::Bool(value)) => Ok(*value),
        _ => Err(damaged()),
    };
    let token = |name: &str| -> Result<String, PlateError> {
        property(prim, name)
            .and_then(text)
            .map(str::to_owned)
            .ok_or_else(damaged)
    };
    let fastener = property(prim, "steel:bolt:fastener")
        .and_then(text)
        .and_then(|t| t.parse::<ContentDigest>().ok())
        .ok_or_else(damaged)?;
    let spacings = match property(prim, "steel:bolt:spacingsMm") {
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| number(v).ok_or_else(damaged))
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(damaged()),
    };
    let targets = prim
        .relationships
        .iter()
        .find(|r| r.name == "bim:connects")
        .map(|r| r.targets.as_slice())
        .unwrap_or_default();
    let part = |path: &String| by_path.get(path.as_str()).copied().ok_or_else(damaged);
    let parts = targets.iter().map(part).collect::<Result<Vec<_>, _>>()?;
    Ok(BoltGroupView {
        id,
        name: property(prim, "bim:name")
            .and_then(text)
            .ok_or_else(damaged)?
            .to_owned(),
        parts,
        spec: GroupSpec {
            fastener,
            point1_mm: vec3("steel:bolt:point1Mm")?,
            point2_mm: vec3("steel:bolt:point2Mm")?,
            plane_normal_mm: vec3("steel:bolt:planeNormalMm")?,
            rotation_rad: number_of("steel:bolt:rotationRad")?,
            on_plane_mm: number_of("steel:bolt:onPlaneMm")?,
            depth_mm: number_of("steel:bolt:depthMm")?,
            offset_start_mm: number_of("steel:bolt:offsetStartMm")?,
            spacing_mode: token("steel:bolt:spacingMode")?,
            spacing_value_mm: number_of("steel:bolt:spacingValueMm")?,
            spacings_mm: spacings,
            length_mode: token("steel:bolt:lengthMode")?,
            length_mm: number_of("steel:bolt:lengthMm")?,
            washer_under_head: flag("steel:bolt:washerUnderHead")?,
            washer_under_nut: flag("steel:bolt:washerUnderNut")?,
            hole_mode: token("steel:bolt:holeMode")?,
            hole_clearance_mm: match property(prim, "steel:bolt:holeClearanceMm") {
                Some(value) => match value {
                    Value::Number(clearance) => Some(*clearance),
                    _ => return Err(damaged()),
                },
                None => None,
            },
        },
    })
}

/// The geometry the placement and hole decisions read: conservative world bounds of every
/// decoded part, its world translation (identity rotation enforced) and the member identities.
type Geometry = (
    BTreeMap<ElementId, Bounds>,
    BTreeMap<ElementId, [f64; 3]>,
    std::collections::BTreeSet<ElementId>,
);

/// Conservative world bounds of every decoded part: a plate contributes the exact box of its
/// contour extruded by its thickness, a member its two world axis points expanded by half the
/// section's bounding-box diagonal in every axis (which contains it under any roll). A part with
/// a non-identity placement rotation is refused as unsupported rather than bounded wrongly.
/// Alongside the bounds it collects every part's world translation and the member identities
/// the hole decision (A14.4) needs.
fn world_bounds(decoded: &crate::plate::Decoded) -> Result<Geometry, PlateError> {
    let unsupported = || {
        PlateError::from(CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "placement.orientation_xyzw",
            "rotated part placement is not supported for bolts in this slice",
        ))
    };
    let identity = |q: [f64; 4]| q[0] == 0.0 && q[1] == 0.0 && q[2] == 0.0;
    let mut bounds = BTreeMap::new();
    let mut translations = BTreeMap::new();
    let mut members = std::collections::BTreeSet::new();
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
        translations.insert(authored.id, translation);
    }
    for (authored, _) in &decoded.members {
        let translation = authored.placement.translation_mm;
        if !identity(authored.placement.orientation_xyzw) {
            return Err(unsupported());
        }
        members.insert(authored.id);
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
        translations.insert(authored.id, translation);
    }
    Ok((bounds, translations, members))
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
        groups: Vec::new(),
        definitions: BTreeMap::new(),
        bounds: BTreeMap::new(),
        translations: BTreeMap::new(),
        members: std::collections::BTreeSet::new(),
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
            // The definition digest discriminates an A14.3 group from an A05 annotation group;
            // the structural validator has already refused a mixed property set.
            Some("connection") if property(prim, "steel:bolt:fastener").is_some() => {
                model.groups.push(bolt_group_of(prim, id, &by_path)?);
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
    model.groups.sort_by_key(|g| g.id);
    let decoded = decode(&reader, &opened, control)?;
    let (bounds, translations, members) = world_bounds(&decoded)?;
    model.bounds = bounds;
    model.translations = translations;
    model.members = members;
    model.definitions = decoded.definitions;
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

/// Whether the pinned model holds a bolt group of this identity, of either form (for
/// `delete-element` and `copy-element`).
pub(crate) fn is_bolt(
    request: &OpenRequest,
    id: &ElementId,
    control: &Control,
) -> Result<bool, PlateError> {
    let model = model_of(request, control)?;
    Ok(model.bolts.iter().any(|b| b.id == *id) || model.groups.iter().any(|g| g.id == *id))
}

/// Whether the host ledger already accepted this operation. A retry of an accepted edit
/// reaches the replay comparison instead of a command-line argument guard, so its content —
/// not its argument shape — decides between `bolt_group_replayed` and `OP_ID_REUSE`.
pub fn op_accepted(store: &RevisionStore, op_id: &OpId) -> Result<bool, PlateError> {
    let op_host = host_uuid(&op_id.to_string())?;
    let (prior, _) = prior_op_with(store, &op_host, || {
        digest_of(b"the accepted request is decided from the operation's revision")
    })?;
    Ok(matches!(prior, PriorOp::Accepted { .. }))
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

/// The placed A14.3 bolt groups of the verified head (empty for a store without a revision).
pub fn bolt_groups(store: &RevisionStore) -> Result<Vec<BoltGroupView>, PlateError> {
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

/// The stored record a command with these inputs and placement writes: the placement decision
/// already carries the expanded spacings, the resolved length and the washer switches, and
/// the hole mode and clearance follow the inputs and the definition (A14.4).
fn expected_spec(
    inputs: &BoltGroupInputs,
    placed: &beamcraft_engine::bolt::BoltGroupPlaced,
) -> GroupSpec {
    let (spacing_mode, spacing_value_mm) = match &inputs.spacing {
        BoltSpacing::Spacings(_) => ("spacings".to_owned(), 0.0),
        BoltSpacing::Pitch { pitch_mm, .. } => ("pitch".to_owned(), *pitch_mm),
        BoltSpacing::OffsetEnd { offset_end_mm, .. } => ("offset_end".to_owned(), *offset_end_mm),
    };
    GroupSpec {
        fastener: inputs.fastener_digest,
        point1_mm: inputs.point1_mm,
        point2_mm: inputs.point2_mm,
        plane_normal_mm: inputs.plane_normal_mm,
        rotation_rad: inputs.rotation_rad,
        on_plane_mm: inputs.on_plane_mm,
        depth_mm: inputs.depth_mm,
        offset_start_mm: inputs.offset_start_mm,
        spacing_mode,
        spacing_value_mm,
        spacings_mm: placed.spacings_mm.clone(),
        length_mode: match inputs.length {
            BoltLength::Auto => "auto".to_owned(),
            BoltLength::Fixed(_) => "fixed".to_owned(),
        },
        length_mm: placed.length_mm,
        washer_under_head: placed.washer_under_head,
        washer_under_nut: placed.washer_under_nut,
        hole_mode: match inputs.resolved_hole_mode() {
            beamcraft_model::fastener::HoleMode::Generate => "generate".to_owned(),
            beamcraft_model::fastener::HoleMode::None => "none".to_owned(),
        },
        hole_clearance_mm: match inputs.resolved_hole_mode() {
            beamcraft_model::fastener::HoleMode::Generate => {
                Some(inputs.resolved_hole_clearance_mm())
            }
            beamcraft_model::fastener::HoleMode::None => None,
        },
    }
}

/// Deterministic record identity of generated hole `index` of the operation (A14.4): the
/// operation's UUIDv7 with its random bits replaced by the hash of the pair. A retry of the
/// same operation authors the same record identities, so the committed result never depends
/// on where fresh ids would come from.
fn derived_op_id(op: OpId, index: usize) -> Result<OpId, PlateError> {
    let mut seed = Vec::with_capacity(24);
    seed.extend_from_slice(op.as_bytes());
    seed.extend_from_slice(&(index as u64).to_le_bytes());
    let digest = beamcraft_model::digest::Sha256::digest(&seed)
        .map_err(|_| integrity("the generated hole identity cannot be hashed"))?;
    let mut bytes = *op.as_bytes();
    for (slot, value) in bytes.iter_mut().skip(6).zip(digest.as_bytes()) {
        *slot = *value;
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x70;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let uuid = beamcraft_model::id::Uuid7::from_bytes(bytes)
        .map_err(|_| integrity("the generated hole identity is not a UUIDv7"))?;
    Ok(OpId::from_uuid(uuid))
}

/// The changes a group's generated holes add (A14.4): the complete new record set and the
/// rewritten mesh of every affected plate, with the footprint writes the records cost. The
/// records named by `keep` are kept; an update passes the filter that drops the group's old
/// records, so regeneration and removal share one path. A hole that does not fit (outside the
/// outline, into another record, past the record or ordinal cap) refuses the whole command
/// and nothing is written.
fn hole_changes(
    op_id: OpId,
    group: ElementId,
    holes: &[beamcraft_engine::bolt::GeneratedHole],
    keep: &dyn Fn(&HoleRecord) -> bool,
    base_request: &OpenRequest,
    control: &Control,
    footprint: Footprint,
) -> Result<(Vec<Change>, Footprint), PlateError> {
    let mut reader = Reader::default();
    let opened = reader.open(base_request.clone(), control)?;
    let decoded = decode(&reader, &opened, control)?;
    let (plates, _) = evaluate_decoded(&decoded)?;
    let materials = material_lookup(&decoded.definitions);
    let mut by_part: BTreeMap<ElementId, Vec<&beamcraft_engine::bolt::GeneratedHole>> =
        BTreeMap::new();
    for hole in holes {
        by_part.entry(hole.part).or_default().push(hole);
    }
    // A group that turns generate off still drops its records: every plate carrying them is
    // rewritten even when no new hole is decided.
    for plate in &plates {
        if plate
            .authored
            .hole_records
            .iter()
            .any(|r| r.owner == Some(group))
        {
            by_part.entry(plate.authored.id).or_default();
        }
    }
    let mut changes = Vec::new();
    let mut footprint = footprint;
    let mut serial = 0usize;
    for (part, part_holes) in by_part {
        let head = plates
            .iter()
            .find(|p| p.authored.id == part)
            .ok_or_else(|| integrity("a generating group named a plate the head does not hold"))?;
        let mut next = head.authored.clone();
        next.hole_records.retain(keep);
        let next_ordinal = decoded
            .hole_history
            .get(&part)
            .map_or(0usize, |h| usize::from(h.next_ordinal));
        for (index, hole) in part_holes.into_iter().enumerate() {
            let ordinal = next_ordinal + index;
            if next.hole_records.len() >= MAX_HOLES {
                return Err(CommandError::new(
                    ErrorCode::LimitExceeded,
                    "hole",
                    "more than 64 holes on one plate",
                )
                .with_rule("DV-LIM-001")
                .into());
            }
            if ordinal > 999 {
                return Err(CommandError::new(
                    ErrorCode::LimitExceeded,
                    "hole.ordinal",
                    "hole ordinal capacity exhausted",
                )
                .into());
            }
            let record_id = derived_op_id(op_id, serial)?;
            serial += 1;
            next.hole_records.push(HoleRecord {
                operation_id: record_id,
                ordinal,
                polygon_mm: hole.polygon_mm.clone(),
                kind: HoleKind::Hole,
                owner: Some(group),
            });
        }
        next.hole_records.sort_by_key(|r| r.ordinal);
        // The replay is the refusal gate: a hole that does not fit refuses the command.
        let body = beamcraft_engine::command::evaluate_plate(&next)?;
        let quantities =
            body.quantities_for(next.material.as_ref().map(&materials).transpose()?)?;
        let record = PlateRecord {
            authored: next.clone(),
            body: body.clone(),
            quantities,
        };
        changes.push(Change::HoleRecords {
            id: part.to_string(),
            records: next.hole_records.clone(),
        });
        changes.push(mesh_change(&part, &body.solid, &plate_record(&record))?);
        footprint = footprint.union(Footprint::for_update(
            part, false, false, true, false, None,
        )?)?;
    }
    Ok((changes, footprint))
}

/// The A14.3 group `id` as the operation committed at `sequence` wrote it, together with the
/// state the revision before it held: the target may have existed as either form (a committed
/// upgrade was an A05 annotation before). `None` when that revision wrote no such group, so
/// the operation id belongs to another request.
fn group_at(
    store: &RevisionStore,
    sequence: u64,
    id: &ElementId,
    control: &Control,
) -> Result<Option<(bool, BoltGroupView)>, PlateError> {
    let at = store.open_at(sequence)?;
    let model = model_of(&open_base(&base_of(&at)?)?, control)?;
    let Some(group) = model.groups.into_iter().find(|g| g.id == *id) else {
        return Ok(None);
    };
    let existed_before = if sequence > 1 {
        let before = store.open_at(sequence - 1)?;
        let before_model = model_of(&open_base(&base_of(&before)?)?, control)?;
        // The edit's target may have existed as either form: a committed upgrade had it as an
        // A05 annotation in the revision before, so a retry of that upgrade must still replay.
        before_model.groups.iter().any(|g| g.id == *id)
            || before_model.bolts.iter().any(|b| b.id == *id)
    } else {
        false
    };
    Ok(Some((existed_before, group)))
}

/// Result of `create_bolt_group` and `update_bolt_group`.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupChange {
    /// True when the same operation was already committed and nothing was written.
    pub replayed: bool,
    /// The revision that holds the result.
    pub revision: RevisionSummary,
    /// The declared footprint; `None` for a replay.
    pub footprint: Option<Footprint>,
    /// Names of the layers whose bytes changed (empty for a replay).
    pub changed_layers: Vec<String>,
    /// The placed groups of the resulting head.
    pub groups: Vec<BoltGroupView>,
}

/// Resolves the definition a command carries: its canonical resource must be exactly the
/// digest the command names, so the stored record and the pinned values cannot diverge.
fn definition_resource(
    definition: &crate::definition::DefinitionRecord,
    command_digest: ContentDigest,
) -> Result<Resource, PlateError> {
    let resource = definition.resource()?;
    if resource.digest != command_digest {
        return Err(integrity(
            "the command does not carry the resolved definition",
        ));
    }
    Ok(resource)
}

/// Authors the A14.3 shape of one command's inputs and placement.
fn layer_group(
    inputs: &BoltGroupInputs,
    placed: &beamcraft_engine::bolt::BoltGroupPlaced,
) -> LayerBoltGroup {
    LayerBoltGroup {
        connects: inputs.parts.iter().map(|part| part.to_string()).collect(),
        fastener: inputs.fastener_digest,
        point1_mm: inputs.point1_mm,
        point2_mm: inputs.point2_mm,
        plane_normal_mm: inputs.plane_normal_mm,
        rotation_rad: inputs.rotation_rad,
        on_plane_mm: inputs.on_plane_mm,
        depth_mm: inputs.depth_mm,
        offset_start_mm: inputs.offset_start_mm,
        spacing_mode: match &inputs.spacing {
            BoltSpacing::Spacings(_) => "spacings".to_owned(),
            BoltSpacing::Pitch { .. } => "pitch".to_owned(),
            BoltSpacing::OffsetEnd { .. } => "offset_end".to_owned(),
        },
        spacing_value_mm: match &inputs.spacing {
            BoltSpacing::Spacings(_) => 0.0,
            BoltSpacing::Pitch { pitch_mm, .. } => *pitch_mm,
            BoltSpacing::OffsetEnd { offset_end_mm, .. } => *offset_end_mm,
        },
        spacings_mm: placed.spacings_mm.clone(),
        length_mode: match inputs.length {
            BoltLength::Auto => "auto".to_owned(),
            BoltLength::Fixed(_) => "fixed".to_owned(),
        },
        length_mm: placed.length_mm,
        washer_under_head: placed.washer_under_head,
        washer_under_nut: placed.washer_under_nut,
        hole_mode: match inputs.resolved_hole_mode() {
            beamcraft_model::fastener::HoleMode::Generate => "generate".to_owned(),
            beamcraft_model::fastener::HoleMode::None => "none".to_owned(),
        },
        hole_clearance_mm: match inputs.resolved_hole_mode() {
            beamcraft_model::fastener::HoleMode::Generate => {
                Some(inputs.resolved_hole_clearance_mm())
            }
            beamcraft_model::fastener::HoleMode::None => None,
        },
    }
}

/// Creates one A14.3 bolt group over 2 to 1024 existing parts and publishes the next revision,
/// or replays the operation. The resolved fastener definition is pinned in the same
/// transaction when the model does not hold it yet (bolt amendment section 6); with
/// `hole_mode = generate` the same transaction also writes one group-owned hole record per
/// crossing fastener on every connected plate (A14.4).
pub fn create_bolt_group(
    store: &RevisionStore,
    context: &BoltContext,
    command: &CreateBoltGroup,
    definition: &crate::definition::DefinitionRecord,
    hook: &mut dyn CommitHook,
) -> Result<GroupChange, PlateError> {
    command.validate_arguments()?;
    let resource = definition_resource(definition, command.inputs.fastener_digest)?;
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
    // The replay comparison needs the placement; a request that no longer places (its parts
    // moved away) is not a replay of anything, so a failure here just means "not the same".
    let expected = || {
        beamcraft_engine::bolt::place_bolt_group(&command.inputs, &model.bounds)
            .map(|placed| expected_spec(&command.inputs, &placed))
            .ok()
    };
    let accepted = match (&prior, &recorded) {
        (PriorOp::Accepted { .. }, Some((_, sequence, _))) => {
            group_at(store, *sequence, &command.new_id, &control)?.map(|(_, group)| group)
        }
        _ => None,
    };
    let same = accepted.as_ref().is_some_and(|group| {
        expected().is_some_and(|expected| {
            group.name == command.name
                && group.parts == command.inputs.parts
                && group.spec == expected
        })
    });
    match prior {
        PriorOp::Accepted { .. } if same => {
            return Ok(GroupChange {
                replayed: true,
                revision: replay_summary(&head, recorded)?,
                footprint: None,
                changed_layers: Vec::new(),
                groups: model.groups,
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
    let (placed, footprint) = model.parts.create_bolt_group(command, &model.bounds)?;
    // The hole decision shares the transaction: generate writes one record per crossing
    // fastener on every plate, none writes no part (A14.4).
    let holes = beamcraft_engine::bolt::generate_holes(
        &command.inputs,
        &placed,
        &model.bounds,
        &model.translations,
        &model.members,
    )?;
    let (hole_writes, footprint) = if holes.is_empty() {
        (Vec::new(), footprint)
    } else {
        hole_changes(
            context.op_id,
            command.new_id,
            &holes,
            &|_: &HoleRecord| true,
            &base_request,
            &control,
            footprint,
        )?
    };
    let schema = schema_identity()?;
    // The definition is pinned in the same transaction unless the model already holds it.
    let mut resources = Vec::new();
    if !model.definitions.contains_key(&resource.digest) {
        resources.push(resource);
    }
    let (mut author, opened) = Author::open(
        base_request,
        EditTargets {
            authored: AUTHORED_LAYER.into(),
            derived: DERIVED_LAYER.into(),
        },
        schema,
        &control,
    )?;
    let mut changes = vec![Change::Create(Element {
        id: command.new_id.to_string(),
        name: command.name.clone(),
        shape: Shape::BoltGroup(layer_group(&command.inputs, &placed)),
        placement: LayerPlacement {
            translation_mm: [0.0; 3],
            orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
        },
        material: None,
    })];
    changes.extend(hole_writes);
    let prepared = author.prepare(
        Prepare {
            context: opened.context.clone(),
            op_id: context.op_id.to_string(),
            actor: context.actor.to_string(),
            schema,
            resources,
            changes,
        },
        &control,
    )?;
    check_footprint(&prepared, &footprint)?;
    let changed_layers = changed_layers(&prepared, &before);
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
    Ok(GroupChange {
        replayed: false,
        revision: summarize(&committed.info),
        footprint: Some(footprint),
        changed_layers,
        groups: bolt_groups(store)?,
    })
}

/// Edits one A14.3 bolt group with the complete new inputs and publishes the next revision as
/// one atomic edit and one undo unit, or replays the operation. A stored A05 annotation group
/// is upgraded by supplying its placement inputs and a fastener; a `repin` supplies the
/// current library entry, which the model then follows.
pub fn update_bolt_group(
    store: &RevisionStore,
    context: &BoltContext,
    command: &UpdateBoltGroup,
    definition: &crate::definition::DefinitionRecord,
    hook: &mut dyn CommitHook,
) -> Result<GroupChange, PlateError> {
    command.validate_arguments()?;
    let resource = definition_resource(definition, command.inputs.fastener_digest)?;
    let control = control();
    let (head, base) = load_base(store, &context.project, &context.workspace)?;
    let before: BTreeMap<String, _> = base
        .assets
        .iter()
        .map(|a| (a.name.clone(), a.digest))
        .collect();
    let base_request = open_base(&base)?;
    let model = model_of(&base_request, &control)?;
    // The target is a placed group of this slice or a stored A05 annotation group (the
    // upgrade of the amendment section 6); either way the identity must exist.
    if !model.groups.iter().any(|g| g.id == command.target_id)
        && !model.bolts.iter().any(|b| b.id == command.target_id)
    {
        return Err(CommandError::new(
            ErrorCode::NotFound,
            "target_id",
            "no bolt group of this identity is in the head revision",
        )
        .into());
    }
    let op_host = host_uuid(&context.op_id.to_string())?;
    let (prior, recorded) = prior_op_with(store, &op_host, || {
        digest_of(b"the accepted request is decided from the operation's revision")
    })?;
    // A request that no longer places is not a replay of anything (see the create).
    let expected = || {
        beamcraft_engine::bolt::place_bolt_group(&command.inputs, &model.bounds)
            .map(|placed| expected_spec(&command.inputs, &placed))
            .ok()
    };
    let accepted = match (&prior, &recorded) {
        (PriorOp::Accepted { .. }, Some((_, sequence, _))) => {
            group_at(store, *sequence, &command.target_id, &control)?
        }
        _ => None,
    };
    let same = accepted.is_some_and(|(existed_before, group)| {
        existed_before
            && expected().is_some_and(|expected| {
                group.parts == command.inputs.parts && group.spec == expected
            })
    });
    match prior {
        PriorOp::Accepted { .. } if same => {
            return Ok(GroupChange {
                replayed: true,
                revision: replay_summary(&head, recorded)?,
                footprint: None,
                changed_layers: Vec::new(),
                groups: model.groups,
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
    let (placed, footprint) = model.parts.update_bolt_group(command, &model.bounds)?;
    // Editing regenerates the holes (bolt amendment section 6): the new decision replaces the
    // group's old records in the same transaction, and turning generate off removes them. A
    // group that never generated and stays quiet touches no part.
    let holes = beamcraft_engine::bolt::generate_holes(
        &command.inputs,
        &placed,
        &model.bounds,
        &model.translations,
        &model.members,
    )?;
    let generated_before = model
        .groups
        .iter()
        .find(|g| g.id == command.target_id)
        .is_some_and(|g| g.spec.hole_mode == "generate");
    let (hole_writes, footprint) = if holes.is_empty() && !generated_before {
        (Vec::new(), footprint)
    } else {
        hole_changes(
            context.op_id,
            command.target_id,
            &holes,
            &|record: &HoleRecord| record.owner != Some(command.target_id),
            &base_request,
            &control,
            footprint,
        )?
    };
    let schema = schema_identity()?;
    let mut resources = Vec::new();
    if !model.definitions.contains_key(&resource.digest) {
        resources.push(resource);
    }
    let (mut author, opened) = Author::open(
        base_request,
        EditTargets {
            authored: AUTHORED_LAYER.into(),
            derived: DERIVED_LAYER.into(),
        },
        schema,
        &control,
    )?;
    let mut changes = vec![Change::BoltGroup {
        id: command.target_id.to_string(),
        group: layer_group(&command.inputs, &placed),
    }];
    changes.extend(hole_writes);
    let prepared = author.prepare(
        Prepare {
            context: opened.context.clone(),
            op_id: context.op_id.to_string(),
            actor: context.actor.to_string(),
            schema,
            resources,
            changes,
        },
        &control,
    )?;
    check_footprint(&prepared, &footprint)?;
    let changed_layers = changed_layers(&prepared, &before);
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
    Ok(GroupChange {
        replayed: false,
        revision: summarize(&committed.info),
        footprint: Some(footprint),
        changed_layers,
        groups: bolt_groups(store)?,
    })
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
    let changed_layers = changed_layers(&prepared, &before);
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
