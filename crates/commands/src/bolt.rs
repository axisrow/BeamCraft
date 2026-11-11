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
//! are only read. The engine ([`steel::engine::bolt`]) decides the identity, the parts' kinds
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
    check_footprint, decode, evaluate_decoded, number, part_mesh_change, prior_op_with, property,
    published_mesh, replay_summary, text,
};
use crate::publication::{PublishRequest, changed_layers, publish_revision};
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::solid::rotate_vector;
use beamcraft_host::revision::{CommitHook, CommitOutcome, RevisionError, RevisionStore, Uuid7};
use beamcraft_layer_engine::authored::{
    Change, Element, Placement as LayerPlacement, Prepare, Resource, schema_identity,
    validate_layers,
};
use beamcraft_layer_engine::{Control, OpenRequest, Reader, Value};
use beamcraft_model::command::CommandError;
use beamcraft_model::command::Placement;
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId};
use session::Refusal;
use session::pipeline::{
    RevisionSummary, base_of, control, digest_of, host_uuid, integrity, load_base, open_base,
    summarize,
};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use steel::engine::bolt::Bounds;
use steel::engine::bolt::Faces;
use steel::engine::committed::PlateRecord;
use steel::engine::transaction::PriorOp;
use steel::engine::weld::{PartKind, Parts};
use steel::layer as steel_layer;
use steel::layer::{Bolt as LayerBolt, BoltGroup as LayerBoltGroup};
use steel::model::command::{
    BoltGroupInputs, BoltLength, BoltSpacing, BoltSpec, CreateBolt, CreateBoltGroup, HoleKind,
    HoleRecord, MAX_HOLES, UpdateBoltGroup,
};

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
    /// The world triangles of every part's stored scene mesh: the face-exact grip geometry
    /// (issue 519 defect B). A part without a usable mesh has no entry and grips per its box.
    faces: BTreeMap<ElementId, Faces>,
    /// The element-local box of every plate (contour extents and the material band of its
    /// `thickness_side`): the frame a generated hole is decided in (A14.4, issue 555).
    slabs: BTreeMap<ElementId, Bounds>,
    /// The world placement of every part: the frame a generated hole's polygon is mapped into
    /// (A14.4); any rotation is carried into the plate-local frame (issue 555).
    placements: BTreeMap<ElementId, Placement>,
    /// Identities of the decoded beams and columns: a generating group refuses them until G14.
    members: std::collections::BTreeSet<ElementId>,
}

fn bolt_of(
    prim: &beamcraft_layer_engine::Prim,
    id: ElementId,
    by_path: &BTreeMap<&str, ElementId>,
) -> Result<BoltView, Refusal> {
    let damaged = || integrity("bolt record is not readable");
    let number_of = |name| property(prim, name).and_then(number).ok_or_else(damaged);
    let vec3 = |name: &str| -> Result<[f64; 3], Refusal> {
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

pub(crate) fn bolt_group_of(
    prim: &beamcraft_layer_engine::Prim,
    id: ElementId,
    by_path: &BTreeMap<&str, ElementId>,
) -> Result<BoltGroupView, Refusal> {
    let damaged = || integrity("bolt group record is not readable");
    let number_of = |name| property(prim, name).and_then(number).ok_or_else(damaged);
    let vec3 = |name: &str| -> Result<[f64; 3], Refusal> {
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
    let token = |name: &str| -> Result<String, Refusal> {
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
/// decoded part, the world triangles of its stored scene mesh (the face-exact grip geometry,
/// issue 519 defect B), its world placement, the member identities and every plate's local box.
type Geometry = (
    BTreeMap<ElementId, Bounds>,
    BTreeMap<ElementId, Faces>,
    BTreeMap<ElementId, Placement>,
    std::collections::BTreeSet<ElementId>,
    BTreeMap<ElementId, Bounds>,
);

/// The world-space triangles of a part's stored scene mesh, turned by its placement. A mesh
/// that is not a usable triangle list is no faces at all: the part then grips per its
/// conservative box, which errs long and never under.
fn part_faces(mesh: &Option<crate::plate::DerivedMesh>, placement: &Placement) -> Option<Faces> {
    let published = published_mesh(mesh.as_ref()?)?;
    let [tx, ty, tz] = placement.translation_mm;
    let world = |v: [f32; 3]| {
        let turned = rotate_vector(
            [f64::from(v[0]), f64::from(v[1]), f64::from(v[2])],
            placement.orientation_xyzw,
        );
        [turned[0] + tx, turned[1] + ty, turned[2] + tz]
    };
    let mut faces = Vec::with_capacity(published.triangles.len());
    for triangle in &published.triangles {
        let point = |i: u32| published.points.get(i as usize).copied();
        let (a, b, c) = (
            point(triangle[0])?,
            point(triangle[1])?,
            point(triangle[2])?,
        );
        faces.push([world(a), world(b), world(c)]);
    }
    Some(faces)
}

/// Conservative world bounds of every decoded part: a plate contributes the exact box of its
/// contour extruded by its thickness on its authored side, turned by its placement; a member its two world axis
/// points expanded by half the section's bounding-box diagonal in every axis (which contains it
/// under any roll and any placement rotation). Alongside the bounds it collects every part's
/// world faces (issue 519 defect B), placement and the member identities the hole decision
/// (A14.4) needs.
fn world_geometry(decoded: &crate::plate::Decoded) -> Result<Geometry, Refusal> {
    let mut bounds = BTreeMap::new();
    let mut faces = BTreeMap::new();
    let mut placements = BTreeMap::new();
    let mut members = std::collections::BTreeSet::new();
    let mut slabs = BTreeMap::new();
    let profiles = profile_lookup(&decoded.definitions);
    for (authored, mesh) in &decoded.plates {
        // A framed (out-of-plane contour) plate carries its material along its contour
        // frame, not the element-local Z this slab and the world bounds assume, so the
        // bearing and generated-hole decisions would read the wrong slab and mis-decide
        // (the review on the 1111 batch). Refuse the group honestly; the frame-aware bolt
        // slice is a documented follow-up.
        if authored.contour_points_mm.iter().any(|p| p[2] != 0.0) {
            return Err(Refusal::new(
                ErrorCode::UnsupportedGeometry,
                "a framed (out-of-plane contour) plate in a bolt group has no slab this \
                 slice reads",
            ));
        }
        let placement = authored.placement;
        let translation = placement.translation_mm;
        // The local box lays the thickness off by the authored side (issue 555 finding 3):
        // one shared definition, never `+ thickness` upwards by assumption.
        let Some([min, max]) = authored.local_box_mm() else {
            return Err(integrity("a committed plate has no contour points"));
        };
        slabs.insert(authored.id, [min, max]);
        // The placement turns the local box: its world bounds contain every turned corner.
        let mut world_min = [f64::INFINITY; 3];
        let mut world_max = [f64::NEG_INFINITY; 3];
        for corner_z in [min[2], max[2]] {
            for corner_y in [min[1], max[1]] {
                for corner_x in [min[0], max[0]] {
                    let turned =
                        rotate_vector([corner_x, corner_y, corner_z], placement.orientation_xyzw);
                    world_min[0] = world_min[0].min(turned[0] + translation[0]);
                    world_min[1] = world_min[1].min(turned[1] + translation[1]);
                    world_min[2] = world_min[2].min(turned[2] + translation[2]);
                    world_max[0] = world_max[0].max(turned[0] + translation[0]);
                    world_max[1] = world_max[1].max(turned[1] + translation[1]);
                    world_max[2] = world_max[2].max(turned[2] + translation[2]);
                }
            }
        }
        bounds.insert(authored.id, [world_min, world_max]);
        if let Some(part_faces) = part_faces(mesh, &placement) {
            faces.insert(authored.id, part_faces);
        }
        placements.insert(authored.id, placement);
    }
    for (authored, mesh) in &decoded.members {
        let placement = authored.placement;
        let translation = placement.translation_mm;
        members.insert(authored.id);
        let (width, height) = profiles(&authored.profile)?.extent_mm();
        // Half the diagonal of the section's bounding box contains every rolled cross-section.
        // The box spans the whole path (one chord for a straight member, the polyline for a
        // polybeam) plus that reach, which any placement rotation keeps exact.
        let reach = 0.5 * (width * width + height * height).sqrt();
        let Some(first) = authored.axis_points_mm.first() else {
            return Err(integrity("a committed member has no path points"));
        };
        let world = |[x, y, z]: [f64; 3]| {
            let turned = rotate_vector([x, y, z], placement.orientation_xyzw);
            [
                turned[0] + translation[0],
                turned[1] + translation[1],
                turned[2] + translation[2],
            ]
        };
        let mut corner = world(*first);
        let mut opposite = corner;
        for p in &authored.axis_points_mm {
            let w = world(*p);
            corner[0] = corner[0].min(w[0]);
            corner[1] = corner[1].min(w[1]);
            corner[2] = corner[2].min(w[2]);
            opposite[0] = opposite[0].max(w[0]);
            opposite[1] = opposite[1].max(w[1]);
            opposite[2] = opposite[2].max(w[2]);
        }
        let expand =
            |[x, y, z]: [f64; 3], sign: f64| [x + sign * reach, y + sign * reach, z + sign * reach];
        let min = expand(corner, -1.0);
        let max = expand(opposite, 1.0);
        bounds.insert(authored.id, [min, max]);
        if let Some(part_faces) = part_faces(mesh, &placement) {
            faces.insert(authored.id, part_faces);
        }
        placements.insert(authored.id, placement);
    }
    Ok((bounds, faces, placements, members, slabs))
}

fn model_of(request: &OpenRequest, control: &Control) -> Result<Model, Refusal> {
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
        placements: BTreeMap::new(),
        members: std::collections::BTreeSet::new(),
        faces: BTreeMap::new(),
        slabs: BTreeMap::new(),
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
    let (bounds, faces, placements, members, slabs) = world_geometry(&decoded)?;
    model.bounds = bounds;
    model.slabs = slabs;
    model.faces = faces;
    model.placements = placements;
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
) -> Result<Option<BoltView>, Refusal> {
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
) -> Result<bool, Refusal> {
    let model = model_of(request, control)?;
    Ok(model.bolts.iter().any(|b| b.id == *id) || model.groups.iter().any(|g| g.id == *id))
}

/// Whether the host ledger already accepted this operation. A retry of an accepted edit
/// reaches the replay comparison instead of a command-line argument guard, so its content —
/// not its argument shape — decides between `bolt_group_replayed` and `OP_ID_REUSE`.
pub fn op_accepted(store: &RevisionStore, op_id: &OpId) -> Result<bool, Refusal> {
    let op_host = host_uuid(&op_id.to_string())?;
    let (prior, _) = prior_op_with(store, &op_host, || {
        digest_of(b"the accepted request is decided from the operation's revision")
    })?;
    Ok(matches!(prior, PriorOp::Accepted { .. }))
}

/// The bolt groups of the verified head (empty for a store without a revision).
pub fn bolts(store: &RevisionStore) -> Result<Vec<BoltView>, Refusal> {
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
pub fn bolt_groups(store: &RevisionStore) -> Result<Vec<BoltGroupView>, Refusal> {
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
    placed: &steel::engine::bolt::BoltGroupPlaced,
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
            steel::model::fastener::HoleMode::Generate => "generate".to_owned(),
            steel::model::fastener::HoleMode::None => "none".to_owned(),
        },
        hole_clearance_mm: match inputs.resolved_hole_mode() {
            steel::model::fastener::HoleMode::Generate => Some(inputs.resolved_hole_clearance_mm()),
            steel::model::fastener::HoleMode::None => None,
        },
    }
}

/// Deterministic record identity of generated hole `index` of the operation (A14.4): the
/// operation's UUIDv7 with its random bits replaced by the hash of the pair. A retry of the
/// same operation authors the same record identities, so the committed result never depends
/// on where fresh ids would come from.
fn derived_op_id(op: OpId, index: usize) -> Result<OpId, Refusal> {
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
    holes: &[steel::engine::bolt::GeneratedHole],
    keep: &dyn Fn(&HoleRecord) -> bool,
    base_request: &OpenRequest,
    control: &Control,
    footprint: Footprint,
) -> Result<(Vec<Change>, Footprint), Refusal> {
    let owned: Vec<_> = holes.iter().map(|hole| (group, hole)).collect();
    regenerated_holes(
        op_id,
        &owned,
        &BTreeSet::from([group]),
        keep,
        base_request,
        control,
        footprint,
        None,
    )
}

/// [`hole_changes`] over several groups at once, each new hole tagged with its owning group,
/// so groups sharing a plate rewrite its record set once (issue 544). `moved` replaces the
/// head's authored record of that plate (a part the same command moves).
#[allow(clippy::too_many_arguments)]
fn regenerated_holes(
    op_id: OpId,
    owned: &[(ElementId, &steel::engine::bolt::GeneratedHole)],
    regenerating: &BTreeSet<ElementId>,
    keep: &dyn Fn(&HoleRecord) -> bool,
    base_request: &OpenRequest,
    control: &Control,
    footprint: Footprint,
    moved: Option<&steel::engine::command::AuthoredPlate>,
) -> Result<(Vec<Change>, Footprint), Refusal> {
    let mut reader = Reader::default();
    let opened = reader.open(base_request.clone(), control)?;
    let decoded = decode(&reader, &opened, control)?;
    let (plates, _) = evaluate_decoded(&decoded)?;
    let materials = material_lookup(&decoded.definitions);
    let mut by_part: BTreeMap<ElementId, Vec<(ElementId, &steel::engine::bolt::GeneratedHole)>> =
        BTreeMap::new();
    for (group, hole) in owned {
        by_part.entry(hole.part).or_default().push((*group, *hole));
    }
    // A group that turns generate off still drops its records: every plate carrying them is
    // rewritten even when no new hole is decided.
    for plate in &plates {
        if plate
            .authored
            .hole_records
            .iter()
            .any(|r| r.owner.is_some_and(|owner| regenerating.contains(&owner)))
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
        let mut next = match moved {
            Some(plate) if plate.id == part => plate.clone(),
            _ => head.authored.clone(),
        };
        next.hole_records.retain(keep);
        let next_ordinal = decoded
            .hole_history
            .get(&part)
            .map_or(0usize, |h| usize::from(h.next_ordinal));
        for (index, (group, hole)) in part_holes.into_iter().enumerate() {
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
        let body = steel::engine::command::evaluate_plate(&next)?;
        let quantities =
            body.quantities_for(next.material.as_ref().map(&materials).transpose()?)?;
        let record = PlateRecord {
            authored: next.clone(),
            body: body.clone(),
            quantities,
        };
        changes.push(steel_layer::hole_records(
            part.to_string(),
            next.hole_records.clone(),
        ));
        changes.push(part_mesh_change(&record)?);
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
) -> Result<Option<(bool, BoltGroupView)>, Refusal> {
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
) -> Result<Resource, Refusal> {
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
    placed: &steel::engine::bolt::BoltGroupPlaced,
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
            steel::model::fastener::HoleMode::Generate => "generate".to_owned(),
            steel::model::fastener::HoleMode::None => "none".to_owned(),
        },
        hole_clearance_mm: match inputs.resolved_hole_mode() {
            steel::model::fastener::HoleMode::Generate => Some(inputs.resolved_hole_clearance_mm()),
            steel::model::fastener::HoleMode::None => None,
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
) -> Result<GroupChange, Refusal> {
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
        steel::engine::bolt::place_bolt_group(&command.inputs, &model.bounds, &model.faces)
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
    let (placed, footprint) =
        model
            .parts
            .create_bolt_group(command, &model.bounds, &model.faces)?;
    // The hole decision shares the transaction: generate writes one record per crossing
    // fastener on every plate, none writes no part (A14.4).
    let holes = steel::engine::bolt::generate_holes(
        &command.inputs,
        &placed.row(),
        &model.slabs,
        &model.placements,
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
    let (mut author, opened) =
        crate::scope::open_author(store, base_request, false, schema, &control)?;
    let mut changes = vec![Change::Create(Element {
        id: command.new_id.to_string(),
        name: command.name.clone(),
        shape: steel_layer::bolt_group(layer_group(&command.inputs, &placed)),
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
) -> Result<GroupChange, Refusal> {
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
        steel::engine::bolt::place_bolt_group(&command.inputs, &model.bounds, &model.faces)
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
                group.parts == command.inputs.parts
                    && group.spec == expected
                    && command
                        .name
                        .as_deref()
                        .is_none_or(|name| group.name == name)
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
    let (placed, footprint) =
        model
            .parts
            .update_bolt_group(command, &model.bounds, &model.faces)?;
    // Editing regenerates the holes (bolt amendment section 6): the new decision replaces the
    // group's old records in the same transaction, and turning generate off removes them. A
    // group that never generated and stays quiet touches no part.
    let holes = steel::engine::bolt::generate_holes(
        &command.inputs,
        &placed.row(),
        &model.slabs,
        &model.placements,
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
    // The scoped author routes this write through the workspace's authorized section/zone
    // layers (U19), the same way creation does; the classic targets are the unscoped shape.
    let (mut author, opened) =
        crate::scope::open_author(store, base_request, false, schema, &control)?;
    let mut changes = vec![steel_layer::replace_bolt_group(
        command.target_id.to_string(),
        layer_group(&command.inputs, &placed),
    )];
    // A rename is label state of the same element (the `bim:name` attribute and the prim's
    // display name), authored in the same transaction as the replaced record.
    if let Some(name) = &command.name {
        changes.push(Change::Label {
            id: command.target_id.to_string(),
            name: name.clone(),
        });
    }
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
) -> Result<BoltChange, Refusal> {
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
    let (mut author, opened) =
        crate::scope::open_author(store, base_request, false, schema, &control)?;
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
                shape: steel_layer::bolt(LayerBolt {
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

/// The bolt groups that follow a part one `update-element` moves (issue 544), as extra
/// changes and footprint of the same edit: the fasteners belong to the group's main part.
///
/// The moved part's world geometry is carried by the rigid delta `new · old⁻¹`. A group whose
/// main part (`parts[0]`) moves follows it rigidly, its row points and plane normal carried by
/// the delta; a group whose other part moves keeps its fasteners where they are. Either way
/// every group over the part is re-placed against the moved geometry and its generated holes
/// are regenerated in the same transaction, so the stored node never comes apart: a part that
/// no longer crosses the row, or a generated hole the engine cannot place (a shank axis too
/// oblique to the moved plate), refuses the whole edit with nothing written. A mirror is not rigid and refuses on a bolted
/// part. `moved_plate` is the moved part's patched authored record when it is a plate.
#[allow(clippy::too_many_arguments)]
pub(crate) fn follow_moved_part(
    base_request: &OpenRequest,
    control: &Control,
    op_id: OpId,
    moved: ElementId,
    old: &Placement,
    new: &Placement,
    mirrored: bool,
    reshaped: bool,
    moved_plate: Option<&steel::engine::command::AuthoredPlate>,
) -> Result<(Vec<Change>, Footprint), Refusal> {
    use steel::engine::transform::{placement_to_local, placement_to_world};
    let mut model = model_of(base_request, control)?;
    let groups: Vec<BoltGroupView> = model
        .groups
        .iter()
        .filter(|g| g.parts.contains(&moved))
        .cloned()
        .collect();
    if groups.is_empty() {
        return Ok((Vec::new(), Footprint::from_entries(&[], &[])?));
    }
    if mirrored {
        return Err(CommandError::new(
            ErrorCode::DependencyConflict,
            "target_id",
            "a bolted part cannot be mirrored; edit or delete its bolt group first",
        )
        .into());
    }
    if reshaped {
        return Err(CommandError::new(
            ErrorCode::DependencyConflict,
            "target_id",
            "move a bolted part and change its shape or material in separate edits",
        )
        .into());
    }
    let carry = |p: [f64; 3]| placement_to_world(new, placement_to_local(old, p));
    let carry_vector = |v: [f64; 3]| {
        let (a, b) = (carry([0.0; 3]), carry(v));
        [b[0] - a[0], b[1] - a[1], b[2] - a[2]]
    };
    // The moved part's world geometry: every face vertex and every corner of its box carried
    // by the delta (the carried box's bounds stay conservative under a rotation).
    if let Some(faces) = model.faces.get_mut(&moved) {
        for triangle in faces.iter_mut() {
            for vertex in triangle.iter_mut() {
                *vertex = carry(*vertex);
            }
        }
    }
    if let Some([min, max]) = model.bounds.get(&moved).copied() {
        let mut low = [f64::INFINITY; 3];
        let mut high = [f64::NEG_INFINITY; 3];
        for x in [min[0], max[0]] {
            for y in [min[1], max[1]] {
                for z in [min[2], max[2]] {
                    let c = carry([x, y, z]);
                    for ((lo, hi), value) in low.iter_mut().zip(high.iter_mut()).zip(c) {
                        *lo = lo.min(value);
                        *hi = hi.max(value);
                    }
                }
            }
        }
        model.bounds.insert(moved, [low, high]);
    }
    model.placements.insert(moved, *new);
    let mut changes = Vec::new();
    let mut footprint = Footprint::from_entries(&[], &[])?;
    let mut owned_holes = Vec::new();
    let mut regenerating = BTreeSet::new();
    for group in &groups {
        let spec = &group.spec;
        let fastener = match model.definitions.get(&spec.fastener) {
            Some(crate::definition::DefinitionRecord::Fastener { spec }) => spec.clone(),
            _ => {
                return Err(integrity(
                    "a committed bolt group's fastener definition is missing",
                ));
            }
        };
        let follows = group.parts.first() == Some(&moved);
        let (point1_mm, point2_mm, plane_normal_mm) = if follows {
            (
                carry(spec.point1_mm),
                carry(spec.point2_mm),
                carry_vector(spec.plane_normal_mm),
            )
        } else {
            (spec.point1_mm, spec.point2_mm, spec.plane_normal_mm)
        };
        let count = spec.spacings_mm.len() + 1;
        let inputs = BoltGroupInputs {
            parts: group.parts.clone(),
            fastener_digest: spec.fastener,
            fastener,
            point1_mm,
            point2_mm,
            plane_normal_mm,
            rotation_rad: spec.rotation_rad,
            on_plane_mm: spec.on_plane_mm,
            depth_mm: spec.depth_mm,
            offset_start_mm: spec.offset_start_mm,
            spacing: match spec.spacing_mode.as_str() {
                "pitch" => BoltSpacing::Pitch {
                    count,
                    pitch_mm: spec.spacing_value_mm,
                },
                "offset_end" => BoltSpacing::OffsetEnd {
                    count,
                    offset_end_mm: spec.spacing_value_mm,
                },
                _ => BoltSpacing::Spacings(spec.spacings_mm.clone()),
            },
            washer_under_head: Some(spec.washer_under_head),
            washer_under_nut: Some(spec.washer_under_nut),
            length: if spec.length_mode == "fixed" {
                BoltLength::Fixed(spec.length_mm)
            } else {
                BoltLength::Auto
            },
            hole_mode: steel::model::fastener::HoleMode::from_token(&spec.hole_mode),
            hole_clearance_mm: spec.hole_clearance_mm,
        };
        let command = UpdateBoltGroup {
            target_id: group.id,
            name: None,
            inputs,
        };
        let (placed, group_footprint) =
            model
                .parts
                .update_bolt_group(&command, &model.bounds, &model.faces)?;
        footprint = footprint.union(group_footprint)?;
        let holes = steel::engine::bolt::generate_holes(
            &command.inputs,
            &placed.row(),
            &model.slabs,
            &model.placements,
            &model.members,
        )?;
        if !holes.is_empty() || spec.hole_mode == "generate" {
            regenerating.insert(group.id);
            owned_holes.extend(holes.into_iter().map(|hole| (group.id, hole)));
        }
        changes.push(steel_layer::replace_bolt_group(
            group.id.to_string(),
            layer_group(&command.inputs, &placed),
        ));
    }
    if !regenerating.is_empty() {
        let owned: Vec<_> = owned_holes.iter().map(|(g, h)| (*g, h)).collect();
        let (hole_writes, next) = regenerated_holes(
            op_id,
            &owned,
            &regenerating,
            &|record: &HoleRecord| !record.owner.is_some_and(|o| regenerating.contains(&o)),
            base_request,
            control,
            footprint,
            moved_plate,
        )?;
        footprint = next;
        changes.extend(hole_writes);
    }
    Ok((changes, footprint))
}
