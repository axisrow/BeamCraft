// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! P01 adapter: the first end-to-end plate, from `CreatePlate` to a committed USD revision and
//! back.
//!
//! The engine (`beamcraft-engine`) validates the command and builds the kernel body; this module
//! is the thin wiring that the module table assigns to the application layer, because only the
//! application may link both the layer engine (USD authoring and reading) and the host
//! (revision store, ledger). It computes no geometry:
//!
//! ```text
//! CreatePlate --engine--> authored state + kernel body + footprint
//!   --layer engine (U02)--> model layer + derived layer bytes (standard USD, no plugin)
//!   --host (U06/U07)--> blobs, intent, verified U03 index, atomic root switch, outcome
//! reopen: verified revision --layer engine (U01/U03)--> authored plates --engine--> snapshot
//! ```
//!
//! # Scope and honesty
//!
//! * One implicit personal workspace per store: project and workspace identities are taken from
//!   the first revision (or supplied for the first one), the generation is the head's plus one.
//!   `CreatePlate` and `CommitWorkspace` are fused: a successful call is a published revision.
//!   Workspaces, drafts and undo (P06, P07) do not exist.
//! * The layer engine runs **in this process**. The owner decision of 2026-10-16 places it in a
//!   separate process (LEW-03 containment); there is no authoring message in the frozen U04
//!   profile, so this path has no process isolation and must not be treated as a hostile-parser
//!   boundary.
//! * Replay (`op_id`): an operation the host ledger proves committed is compared with the
//!   element of that id in the head revision. This is sound only while no command edits or
//!   deletes elements (true in P01); the stored canonical request of F05 replaces it.
//! * Quantities are returned, not stored: the pinned U02 schema has no quantity attributes.

use crate::definition::{
    DefinitionRecord, decode_definitions, definition_digest, material_lookup, profile_lookup,
};
use crate::level::levels_of;
use crate::marks::marks_of;
use beamcraft_engine::command::AuthoredPlate;
use beamcraft_engine::footprint::{Entry, Footprint, Group, Target};
use beamcraft_engine::level::Levels;
use beamcraft_engine::marks::Numbering;
use beamcraft_engine::member::AuthoredMember;
use beamcraft_engine::quantity::{
    Assessment, Freshness, QuantityRecord, StoredRecord, assess, is_superseded_algorithm,
};
use beamcraft_engine::query::committed::{
    MemberRecord, PlateRecord, evaluate_committed, evaluate_committed_members, ids,
    scene_elements_with_members,
};
use beamcraft_engine::solid::SolidMesh;
use beamcraft_engine::transaction::{Admission, PriorOp, admit_create_plate};
use beamcraft_engine::{Limits, ViewPin, ViewSnapshot};
use beamcraft_host::ledger::Outcome;
use beamcraft_host::revision::{
    CommitHook, CommitOutcome, Committed, ContentRejection, ContentVerifier, ExpectedBase,
    IndexEvidence, LayerEntry, LayerRole, Revision, RevisionDraft, RevisionError, RevisionInfo,
    RevisionStore, Uuid7, VerifyRequest,
};
use beamcraft_layer_engine::authored::{
    Author, Change, EditTargets, Element, Group as LayerGroup, PROJECT_RECORD_KEY,
    Placement as LayerPlacement, Prepare, Prepared, SchemaIdentity, Shape, StoredQuantities,
    project_record, schema_identity, validate_layers,
};
use beamcraft_layer_engine::index::Completeness;
use beamcraft_layer_engine::{
    Asset, BACKEND, Budgets, Cancellation, Control, Diagnostic, Identity, OpenRequest, Opened,
    Prim, Profile, Reader, Value,
};
use beamcraft_model::command::{CommandError, CreatePlate, MemberKind};
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId};
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

/// Virtual name of the root layer.
pub const ROOT_LAYER: &str = "root.usda";
/// Virtual name of the authored (model) layer.
pub const AUTHORED_LAYER: &str = "authored.usda";
/// Virtual name of the derived (geometry) layer.
pub const DERIVED_LAYER: &str = "derived.usda";

const EMPTY_LAYER: &str = "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n)\n";

/// The root layer of the first revision of a project. It records the ProjectId (D1) in its
/// `customLayerData`, once: later revisions keep it, because the layer engine preserves the other
/// entries of that dictionary on every edit and no command writes it.
fn empty_root(project: &ProjectId) -> String {
    format!(
        "#usda 1.0\n(\n    customLayerData = {{\n        string {PROJECT_RECORD_KEY} = \"{project}\"\n    }}\n    defaultPrim = \"World\"\n    metersPerUnit = 0.001\n    subLayers = [@authored.usda@, @derived.usda@]\n    upAxis = \"Z\"\n)\n"
    )
}

/// A typed failure: a stable F04 code and a fixed safe message. No paths, no OS text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlateError {
    /// Stable F04 code.
    pub code: ErrorCode,
    /// Fixed ASCII explanation.
    pub message: &'static str,
}

impl PlateError {
    const fn new(code: ErrorCode, message: &'static str) -> Self {
        Self { code, message }
    }
}

impl From<CommandError> for PlateError {
    fn from(error: CommandError) -> Self {
        Self::new(error.code, error.message)
    }
}

impl From<Diagnostic> for PlateError {
    fn from(error: Diagnostic) -> Self {
        Self::new(error.code, error.message)
    }
}

impl From<ContentRejection> for PlateError {
    fn from(error: ContentRejection) -> Self {
        Self::new(error.code, error.message)
    }
}

impl From<RevisionError> for PlateError {
    fn from(error: RevisionError) -> Self {
        match error {
            RevisionError::Content(rejection) => rejection.into(),
            other => Self::new(other.code(), "revision store rejected the operation"),
        }
    }
}

impl std::fmt::Display for PlateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for PlateError {}

pub(crate) fn integrity(message: &'static str) -> PlateError {
    PlateError::new(ErrorCode::IntegrityError, message)
}

pub(crate) fn control() -> Control {
    Control {
        cancellation: Cancellation::default(),
        deadline: Instant::now() + Duration::from_secs(60),
    }
}

pub(crate) fn digest_of(bytes: &[u8]) -> Result<ContentDigest, PlateError> {
    Sha256::digest(bytes).map_err(|_| PlateError::new(ErrorCode::LimitExceeded, "layer too large"))
}

pub(crate) fn host_uuid(text: &str) -> Result<Uuid7, PlateError> {
    Uuid7::parse(text).map_err(PlateError::from)
}

/// The pin of the context that has no base revision.
fn initial_pin() -> Result<ContentDigest, PlateError> {
    digest_of(b"beamcraft-p01-initial-base")
}

fn role_of(name: &str) -> LayerRole {
    if name == DERIVED_LAYER {
        LayerRole::Derived
    } else {
        LayerRole::Authored
    }
}

pub(crate) fn open_request(
    root: &str,
    project: &str,
    workspace: &str,
    revision: ContentDigest,
    generation: u64,
    assets: Vec<Asset>,
    external: &BTreeSet<String>,
) -> Result<OpenRequest, PlateError> {
    // The host selects ownership: every layer of a revision is owned except the external ones
    // (foreign sources), which stay read-only whatever their bytes say.
    let owned_layers = assets
        .iter()
        .map(|a| a.name.clone())
        .filter(|name| !external.contains(name))
        .collect();
    let mut request = OpenRequest {
        identity: Identity {
            project: project.into(),
            workspace: workspace.into(),
            revision,
            generation,
        },
        root: root.into(),
        assets,
        owned_layers,
        manifest: digest_of(b"placeholder")?,
        backend: BACKEND,
        profile: Profile::StaticLocal,
        budgets: Budgets::default(),
    };
    request.manifest = request.manifest_digest()?;
    Ok(request)
}

fn asset(name: &str, bytes: &[u8]) -> Result<Asset, PlateError> {
    Ok(Asset {
        name: name.into(),
        digest: digest_of(bytes)?,
        bytes: bytes.to_vec(),
    })
}

/// Derived mesh as read back from a derived layer.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DerivedMesh {
    pub(crate) points: Vec<[f32; 3]>,
    pub(crate) counts: Vec<i64>,
    pub(crate) indices: Vec<i64>,
    shape_key: String,
    orientation: String,
    subdivision: String,
    /// The quantity record stored beside the mesh, if any. Not part of the geometry.
    stored: Option<StoredRecord>,
}

impl DerivedMesh {
    /// Whether two meshes are the same geometry, whatever quantity record they carry.
    fn same_geometry(&self, other: &Self) -> bool {
        self.points == other.points
            && self.counts == other.counts
            && self.indices == other.indices
            && self.shape_key == other.shape_key
            && self.orientation == other.orientation
            && self.subdivision == other.subdivision
    }
}

/// How a prim authors its placement (E02: a proxy's pose is read from here).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlacementRead {
    /// The prim authors no transform at all.
    Absent,
    /// Exactly a translation and an orientation, as the F04 pose.
    Rigid(beamcraft_model::command::Placement),
    /// Any other transform form (a scale, a different operation order, a missing half).
    Other,
}

/// The published mesh of an element the kernel does not know (E02: kept for its proxy). Triangles
/// only, indices checked against the points; anything else is not a usable mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct PublishedMesh {
    /// Points in their stored order, element-local millimetres.
    pub points: Vec<[f32; 3]>,
    /// Zero-based vertex indices of each triangle in their stored order.
    pub triangles: Vec<[u32; 3]>,
}

fn published_mesh(mesh: &DerivedMesh) -> Option<PublishedMesh> {
    if mesh.points.is_empty()
        || mesh.counts.is_empty()
        || mesh.counts.iter().any(|c| *c != 3)
        || mesh.indices.len() != mesh.counts.len().checked_mul(3)?
        || mesh.points.iter().flatten().any(|v| !v.is_finite())
    {
        return None;
    }
    let triangles = mesh
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|&[a, b, c]| {
            let index = |v: i64| {
                u32::try_from(v)
                    .ok()
                    .filter(|i| usize::try_from(*i).is_ok_and(|i| i < mesh.points.len()))
            };
            Some([index(a)?, index(b)?, index(c)?])
        })
        .collect::<Option<Vec<_>>>()?;
    Some(PublishedMesh {
        points: mesh.points.clone(),
        triangles,
    })
}

/// The common facts of one element prim, whatever its kind (E02: kinds without a mapping are
/// exported as proxies and need a name, a pose and the lifecycle). Levels are not listed.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementMeta {
    /// The `bim:elementKind` token, `unknown` when the prim authors none.
    pub kind: String,
    /// `bim:name`, when authored as a string.
    pub name: Option<String>,
    /// `bim:schemaVersion`, when authored.
    pub schema_version: Option<String>,
    /// `bim:geometryMode`, when authored.
    pub geometry_mode: Option<String>,
    /// Whether `bim:lifecycle` is `deactivated` or the prim is inactive.
    pub deactivated: bool,
    /// The authored placement.
    pub placement: PlacementRead,
    /// The assembly this element is a part of (`bim:partOf`).
    pub part_of: Option<ElementId>,
    /// The published mesh of an element whose kind the kernel does not know, when it has a usable
    /// one. Plates, beams and columns carry `None`: their mesh is the kernel's.
    pub mesh: Option<PublishedMesh>,
}

fn placement_read(prim: &Prim) -> PlacementRead {
    let translation = property(prim, "xformOp:translate").and_then(numbers::<3>);
    let orient = property(prim, "xformOp:orient").and_then(numbers::<4>);
    let order = property(prim, "xformOpOrder");
    let order_is_pose = match order {
        Some(Value::Array(items)) => {
            items.iter().filter_map(text).collect::<Vec<_>>()
                == ["xformOp:translate", "xformOp:orient"]
        }
        _ => false,
    };
    let other_ops = prim.properties.iter().any(|p| {
        p.name.starts_with("xformOp:")
            && !matches!(p.name.as_str(), "xformOp:translate" | "xformOp:orient")
    });
    match (translation, orient, order.is_some(), other_ops) {
        (None, None, false, false) => PlacementRead::Absent,
        (Some(t), Some([w, x, y, z]), true, false) if order_is_pose => {
            PlacementRead::Rigid(beamcraft_model::command::Placement {
                translation_mm: t,
                orientation_xyzw: [x, y, z, w],
            })
        }
        _ => PlacementRead::Other,
    }
}

fn element_meta(
    prim: &Prim,
    kind: &str,
    by_path: &BTreeMap<&str, ElementId>,
    mesh: Option<PublishedMesh>,
) -> ElementMeta {
    let part_of = prim
        .relationships
        .iter()
        .find(|r| r.name == "bim:partOf")
        .and_then(|r| match r.targets.as_slice() {
            [target] => by_path.get(target.as_str()).copied(),
            _ => None,
        });
    ElementMeta {
        kind: kind.to_owned(),
        name: property(prim, "bim:name").and_then(text).map(str::to_owned),
        schema_version: property(prim, "bim:schemaVersion")
            .and_then(text)
            .map(str::to_owned),
        geometry_mode: property(prim, "bim:geometryMode")
            .and_then(text)
            .map(str::to_owned),
        deactivated: !prim.active
            || property(prim, "bim:lifecycle").and_then(text) == Some("deactivated"),
        placement: placement_read(prim),
        part_of,
        mesh,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Decoded {
    plates: Vec<(AuthoredPlate, Option<DerivedMesh>)>,
    pub(crate) members: Vec<(AuthoredMember, Option<DerivedMesh>)>,
    other_elements: usize,
    pub(crate) definitions: BTreeMap<ContentDigest, DefinitionRecord>,
    /// The levels of the model and the level of every plate, beam and column (D2).
    pub(crate) levels: Levels,
    /// The common facts of every element except levels, by id.
    pub(crate) elements: BTreeMap<ElementId, ElementMeta>,
    /// The marks of the model and their approval states (A08).
    pub(crate) marks: Numbering,
}

pub(crate) fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => Some(*n),
        _ => None,
    }
}

fn integer(value: &Value) -> Option<i64> {
    match value {
        Value::Int(n) => Some(*n),
        Value::Uint(n) => i64::try_from(*n).ok(),
        _ => None,
    }
}

fn numbers<const N: usize>(value: &Value) -> Option<[f64; N]> {
    let Value::Array(items) = value else {
        return None;
    };
    let parsed: Vec<f64> = items.iter().map(number).collect::<Option<_>>()?;
    parsed.try_into().ok()
}

pub(crate) fn text(value: &Value) -> Option<&str> {
    match value {
        Value::Text(t) => Some(t),
        _ => None,
    }
}

pub(crate) fn property<'a>(prim: &'a Prim, name: &str) -> Option<&'a Value> {
    prim.properties
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| p.value.as_ref())
}

fn decode_plate(prim: &Prim, id: ElementId) -> Result<AuthoredPlate, PlateError> {
    let bad = || integrity("committed plate lacks a required authored input");
    let name = property(prim, "bim:name").and_then(text).ok_or_else(bad)?;
    let thickness = property(prim, "steel:plate:thicknessMm")
        .and_then(number)
        .ok_or_else(bad)?;
    let Some(Value::Array(points)) = property(prim, "steel:plate:contourPointsMm") else {
        return Err(bad());
    };
    let contour = points
        .iter()
        .map(numbers::<3>)
        .collect::<Option<Vec<_>>>()
        .ok_or_else(bad)?;
    let translation = property(prim, "xformOp:translate")
        .and_then(numbers::<3>)
        .ok_or_else(bad)?;
    let [w, x, y, z] = property(prim, "xformOp:orient")
        .and_then(numbers::<4>)
        .ok_or_else(bad)?;
    let material = decode_material(prim)?;
    Ok(AuthoredPlate {
        id,
        name: name.to_owned(),
        contour_points_mm: contour,
        thickness_mm: thickness,
        placement: beamcraft_model::command::Placement {
            translation_mm: translation,
            orientation_xyzw: [x, y, z, w],
        },
        material,
        holes_mm: decode_holes(prim)?,
    })
}

/// The through-hole polygons of a committed plate: the flat point array cut by the vertex counts.
/// Both attributes or neither; every count is at least three and every `z` is zero.
fn decode_holes(prim: &Prim) -> Result<Vec<Vec<[f64; 2]>>, PlateError> {
    decode_polygons(
        prim,
        "steel:plate:holePointsMm",
        "steel:plate:holeVertexCounts",
        beamcraft_model::command::MAX_HOLES,
        "committed plate has inconsistent hole attributes",
    )
}

/// Planar polygons stored as one flat `double3[]` point array cut by an `int[]` of vertex counts:
/// both attributes or neither (none is an empty list); every `z` is zero and the counts add up to
/// the points. `max_groups` only bounds the up-front allocation.
pub(crate) fn decode_polygons(
    prim: &Prim,
    points_name: &str,
    counts_name: &str,
    max_groups: usize,
    message: &'static str,
) -> Result<Vec<Vec<[f64; 2]>>, PlateError> {
    let bad = || integrity(message);
    // A declaration without a value is not "no polygons": it is a damaged record.
    let declared = |name: &str| prim.properties.iter().any(|p| p.name == name);
    let (points, counts) = match (property(prim, points_name), property(prim, counts_name)) {
        (None, None) if !declared(points_name) && !declared(counts_name) => {
            return Ok(Vec::new());
        }
        (Some(Value::Array(points)), Some(Value::Array(counts))) => (points, counts),
        _ => return Err(bad()),
    };
    let points: Vec<[f64; 3]> = points
        .iter()
        .map(numbers::<3>)
        .collect::<Option<_>>()
        .ok_or_else(bad)?;
    let mut groups = Vec::with_capacity(counts.len().min(max_groups));
    let mut at = 0usize;
    for count in counts {
        let n = usize::try_from(integer(count).ok_or_else(bad)?).map_err(|_| bad())?;
        let end = at.checked_add(n).ok_or_else(bad)?;
        let group = points.get(at..end).ok_or_else(bad)?;
        if group.iter().any(|p| p[2] != 0.0) {
            return Err(bad());
        }
        groups.push(group.iter().map(|p| [p[0], p[1]]).collect());
        at = end;
    }
    if at != points.len() {
        return Err(bad());
    }
    Ok(groups)
}

/// One planar polygon stored as a `double3[]` with every `z` zero.
pub(crate) fn decode_polygon(
    prim: &Prim,
    name: &str,
    message: &'static str,
) -> Result<Vec<[f64; 2]>, PlateError> {
    let bad = || integrity(message);
    let Some(Value::Array(points)) = property(prim, name) else {
        return Err(bad());
    };
    points
        .iter()
        .map(|p| match numbers::<3>(p) {
            Some([x, y, 0.0]) => Some([x, y]),
            _ => None,
        })
        .collect::<Option<_>>()
        .ok_or_else(bad)
}

/// The one pinned material target of an element: none, or exactly one definition path. An
/// explicit empty target list and an absent relationship both mean the material is unknown.
fn decode_material(prim: &Prim) -> Result<Option<ContentDigest>, PlateError> {
    let Some(relationship) = prim.relationships.iter().find(|r| r.name == "bim:material") else {
        return Ok(None);
    };
    match relationship.targets.as_slice() {
        [] => Ok(None),
        [target] => Ok(Some(definition_digest(target).ok_or_else(|| {
            integrity("material target is not an immutable definition")
        })?)),
        _ => Err(integrity("plate has more than one material target")),
    }
}

fn decode_mesh(prim: &Prim) -> Option<DerivedMesh> {
    let Value::Array(points) = property(prim, "points")? else {
        return None;
    };
    let Value::Array(counts) = property(prim, "faceVertexCounts")? else {
        return None;
    };
    let Value::Array(indices) = property(prim, "faceVertexIndices")? else {
        return None;
    };
    let points = points
        .iter()
        .map(|p| {
            let [x, y, z] = numbers::<3>(p)?;
            Some([x as f32, y as f32, z as f32])
        })
        .collect::<Option<Vec<_>>>()?;
    Some(DerivedMesh {
        points,
        counts: counts.iter().map(integer).collect::<Option<_>>()?,
        indices: indices.iter().map(integer).collect::<Option<_>>()?,
        shape_key: property(prim, "bim:shapeKey").and_then(text)?.to_owned(),
        orientation: property(prim, "orientation").and_then(text)?.to_owned(),
        subdivision: property(prim, "subdivisionScheme")
            .and_then(text)?
            .to_owned(),
        stored: decode_stored(prim)?,
    })
}

/// The quantity record stored beside a mesh: absent (`Some(None)`), complete (`Some(Some(..))`)
/// or malformed (`None`). A record is absent only when none of its attributes exist; otherwise
/// every field must be there with its USD type, and a mass value only with a `current` mass.
fn decode_stored(prim: &Prim) -> Option<Option<StoredRecord>> {
    let typed = |name: &str, ty: &str| -> Option<Option<&Value>> {
        let Some(p) = prim.properties.iter().find(|p| p.name == name) else {
            return Some(None);
        };
        match &p.value {
            Some(v) if p.type_name == ty && !matches!(v, Value::Block) => Some(Some(v)),
            _ => None,
        }
    };
    // Absent only when no attribute of the record exists; a partial record is damaged, so
    // removing the signature cannot make a tampered value look like a missing record.
    const FIELDS: [&str; 7] = [
        "bim:volumeMm3",
        "bim:volumeStatus",
        "bim:massKg",
        "bim:massStatus",
        "bim:shapeSignature",
        "bim:massSignature",
        "bim:quantityAlgorithm",
    ];
    if !prim
        .properties
        .iter()
        .any(|p| FIELDS.contains(&p.name.as_str()))
    {
        return Some(None);
    }
    let string = |name: &str, ty: &str| -> Option<&str> { text(typed(name, ty)??) };
    let digest = |name: &str| -> Option<ContentDigest> {
        string(name, "string")?.parse::<ContentDigest>().ok()
    };
    let mass_kg = match typed("bim:massKg", "double")? {
        None => None,
        Some(value) => Some(number(value)?),
    };
    let record = StoredRecord {
        volume_mm3: number(typed("bim:volumeMm3", "double")??)?,
        volume_status: string("bim:volumeStatus", "token")?.to_owned(),
        mass_kg,
        mass_status: string("bim:massStatus", "token")?.to_owned(),
        shape_signature: digest("bim:shapeSignature")?,
        mass_signature: digest("bim:massSignature")?,
        algorithm: string("bim:quantityAlgorithm", "string")?.to_owned(),
    };
    Some(Some(record))
}

/// Reads every prim of the pinned stage and decodes the plates and their derived meshes.
pub(crate) fn decode(
    reader: &Reader,
    opened: &Opened,
    control: &Control,
) -> Result<Decoded, PlateError> {
    let mut prims = Vec::new();
    let mut cursor = None;
    loop {
        let page = reader.read(&opened.context, cursor.as_ref(), control)?;
        prims.extend(page.items);
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    let by_path: BTreeMap<&str, &Prim> = prims.iter().map(|p| (p.path.as_str(), p)).collect();
    let mut plates = Vec::new();
    let mut members = Vec::new();
    let mut other = 0usize;
    let mut elements = BTreeMap::new();
    let ids: BTreeMap<&str, ElementId> = prims
        .iter()
        .filter_map(|p| {
            Some((
                p.path.as_str(),
                ElementId::parse(p.element_id.as_deref()?).ok()?,
            ))
        })
        .collect();
    for prim in &prims {
        let Some(text_id) = prim.element_id.as_deref() else {
            continue;
        };
        let id = ElementId::parse(text_id).map_err(|_| integrity("element id is not a UUIDv7"))?;
        if prim.path != id.prim_path() {
            return Err(integrity("element path does not match its id"));
        }
        let mesh = || {
            by_path
                .get(format!("{}/Geometry", prim.path).as_str())
                .and_then(|m| decode_mesh(m))
        };
        let kind = property(prim, "bim:elementKind").and_then(text);
        if kind != Some("level") {
            let proxy_mesh = match kind {
                Some("plate" | "beam" | "column") => None,
                _ => mesh().as_ref().and_then(published_mesh),
            };
            elements.insert(
                id,
                element_meta(prim, kind.unwrap_or("unknown"), &ids, proxy_mesh),
            );
        }
        match kind {
            Some("plate") => plates.push((decode_plate(prim, id)?, mesh())),
            Some("beam") => members.push((decode_member(prim, id, MemberKind::Beam)?, mesh())),
            Some("column") => {
                members.push((decode_member(prim, id, MemberKind::Column)?, mesh()));
            }
            // R01 wrappers are decoded by `beamcraft_cli::reference`.
            Some("reference") => {}
            _ => other += 1,
        }
    }
    let definitions = decode_definitions(&prims)?;
    let levels = levels_of(&prims)?;
    let marks = marks_of(&prims)?;
    Ok(Decoded {
        plates,
        members,
        other_elements: other,
        definitions,
        levels,
        elements,
        marks,
    })
}

/// A committed member's authored inputs. The profile is one required pinned definition target;
/// an absent or empty profile relationship is an integrity error, never a default.
fn decode_member(
    prim: &Prim,
    id: ElementId,
    kind: MemberKind,
) -> Result<AuthoredMember, PlateError> {
    let bad = || integrity("committed member lacks a required authored input");
    let name = property(prim, "bim:name").and_then(text).ok_or_else(bad)?;
    let Some(Value::Array(points)) = property(prim, "bim:axisPointsMm") else {
        return Err(bad());
    };
    let axis: Vec<[f64; 3]> = points
        .iter()
        .map(numbers::<3>)
        .collect::<Option<_>>()
        .ok_or_else(bad)?;
    let axis_points_mm: [[f64; 3]; 2] = axis.try_into().map_err(|_| bad())?;
    let roll_rad = property(prim, "steel:member:rollRad")
        .map(|v| number(v).ok_or_else(bad))
        .transpose()?;
    let profile = match prim
        .relationships
        .iter()
        .find(|r| r.name == "bim:profile")
        .map(|r| r.targets.as_slice())
    {
        Some([target]) => definition_digest(target)
            .ok_or_else(|| integrity("profile target is not an immutable definition"))?,
        _ => return Err(bad()),
    };
    let translation = property(prim, "xformOp:translate")
        .and_then(numbers::<3>)
        .ok_or_else(bad)?;
    let [w, x, y, z] = property(prim, "xformOp:orient")
        .and_then(numbers::<4>)
        .ok_or_else(bad)?;
    Ok(AuthoredMember {
        id,
        kind,
        name: name.to_owned(),
        axis_points_mm,
        profile,
        roll_rad,
        placement: beamcraft_model::command::Placement {
            translation_mm: translation,
            orientation_xyzw: [x, y, z, w],
        },
        material: decode_material(prim)?,
    })
}

pub(crate) fn kernel_mesh(solid: &SolidMesh) -> Result<DerivedMesh, PlateError> {
    let points = solid
        .vertices()
        .iter()
        .map(|p| {
            let cast = [p[0] as f32, p[1] as f32, p[2] as f32];
            if cast.iter().all(|v| v.is_finite()) {
                Ok(cast)
            } else {
                Err(PlateError::new(
                    ErrorCode::UnsupportedGeometry,
                    "mesh coordinate does not fit float32",
                ))
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    // Far from the element origin float32 cannot tell nearby vertices apart; a mesh that loses
    // vertices that way would store a different (even flat) solid than the one measured.
    let distinct = |bits: &mut std::collections::BTreeSet<[u64; 3]>, p: [f64; 3]| {
        bits.insert(p.map(|v| (v + 0.0).to_bits()));
    };
    let (mut exact, mut stored) = (
        std::collections::BTreeSet::new(),
        std::collections::BTreeSet::new(),
    );
    for (p, cast) in solid.vertices().iter().zip(&points) {
        distinct(&mut exact, *p);
        distinct(&mut stored, cast.map(f64::from));
    }
    if exact.len() != stored.len() {
        return Err(PlateError::new(
            ErrorCode::UnsupportedGeometry,
            "mesh coordinates collapse in float32; the element is too far from its origin",
        ));
    }
    Ok(DerivedMesh {
        points,
        counts: vec![3; solid.triangles().len()],
        indices: solid
            .triangles()
            .iter()
            .flatten()
            .map(|i| i64::from(*i))
            .collect(),
        shape_key: solid.digest().to_string(),
        orientation: "rightHanded".into(),
        subdivision: "none".into(),
        stored: None,
    })
}

/// Re-evaluates every committed plate and member with the kernel, resolving their pinned
/// definitions from the same stage.
pub(crate) fn evaluate_decoded(
    decoded: &Decoded,
) -> Result<(Vec<PlateRecord>, Vec<MemberRecord>), PlateError> {
    let authored: Vec<AuthoredPlate> = decoded.plates.iter().map(|(a, _)| a.clone()).collect();
    let members: Vec<AuthoredMember> = decoded.members.iter().map(|(a, _)| a.clone()).collect();
    let materials = material_lookup(&decoded.definitions);
    let plates = evaluate_committed(&authored, &materials)?;
    let members =
        evaluate_committed_members(&members, &profile_lookup(&decoded.definitions), &materials)?;
    Ok((plates, members))
}

/// The quantity record the engine expects for a plate: its quantities under the signature of its
/// shape inputs.
pub(crate) fn plate_record(record: &PlateRecord) -> QuantityRecord {
    beamcraft_engine::quantity::plate_record(
        &record.authored,
        record.body.algorithm(),
        &record.quantities,
    )
}

/// The quantity record the engine expects for a member.
pub(crate) fn member_record(record: &MemberRecord) -> QuantityRecord {
    beamcraft_engine::quantity::member_record(
        &record.authored,
        &record.body.profile,
        &record.quantities,
    )
}

/// What the derived layer stores for a quantity record, and what a reader found.
pub(crate) fn stored_quantities(record: &QuantityRecord) -> StoredQuantities {
    StoredQuantities {
        volume_mm3: record.volume_mm3,
        volume_status: record.volume_status.to_owned(),
        mass_kg: record.mass_kg,
        mass_status: record.mass_status.to_owned(),
        shape_signature: record.shape_signature,
        mass_signature: record.mass_signature,
        algorithm: record.algorithm.clone(),
    }
}

/// The stored record of one element next to what the engine computes now, and how fresh it is.
#[derive(Debug, Clone, PartialEq)]
pub struct QuantityReport {
    /// What the derived layer stores, if anything.
    pub stored: Option<StoredRecord>,
    /// What the engine computes from the present inputs.
    pub expected: QuantityRecord,
    /// Freshness of the stored volume and mass.
    pub assessment: Assessment,
}

/// The committed derived mesh of every plate and member must be the kernel's mesh of its
/// authored inputs, and a stored quantity record must be the engine's or, outside `strict` mode,
/// is reported as stale. Publishing is strict: a layer whose stored quantities disagree with the
/// kernel cannot become a revision.
fn verify_derived(
    records: &[PlateRecord],
    member_records: &[MemberRecord],
    decoded: &Decoded,
    strict: bool,
) -> Result<BTreeMap<ElementId, QuantityReport>, PlateError> {
    let found: BTreeMap<ElementId, &Option<DerivedMesh>> = decoded
        .plates
        .iter()
        .map(|(a, m)| (a.id, m))
        .chain(decoded.members.iter().map(|(a, m)| (a.id, m)))
        .collect();
    let expected = records
        .iter()
        .map(|r| (r.authored.id, &r.body.solid, plate_record(r), false))
        .chain(
            member_records
                .iter()
                .map(|r| (r.authored.id, &r.body.solid, member_record(r), true)),
        );
    let mut reports = BTreeMap::new();
    for (id, solid, record, is_member) in expected {
        let mesh = kernel_mesh(solid)?;
        let Some(Some(actual)) = found.get(&id) else {
            return Err(integrity("element has no derived mesh"));
        };
        if !actual.same_geometry(&mesh) {
            return Err(integrity("derived mesh differs from the kernel result"));
        }
        let assessment = assess(actual.stored.as_ref(), &record);
        // A record that an earlier slice wrote for a member (a superseded algorithm label) is a
        // stale value to recompute, not a layer that cannot be published. Plate algorithms did
        // not change, so a plate carrying such a label is a wrong record like any other.
        let superseded = is_member
            && actual
                .stored
                .as_ref()
                .is_some_and(|s| is_superseded_algorithm(&s.algorithm));
        if strict
            && actual.stored.is_some()
            && !superseded
            && (assessment.volume != Freshness::Current || assessment.mass != Freshness::Current)
        {
            return Err(integrity("stored quantities differ from the kernel's"));
        }
        reports.insert(
            id,
            QuantityReport {
                stored: actual.stored.clone(),
                expected: record,
                assessment,
            },
        );
    }
    Ok(reports)
}

fn evidence(
    index: &beamcraft_layer_engine::index::ElementIndex,
    root_layer: &str,
    schema: ContentDigest,
) -> IndexEvidence {
    let token = index.token();
    IndexEvidence {
        project: token.identity.project.clone(),
        workspace: token.identity.workspace.clone(),
        generation: token.identity.generation,
        pinned_revision: token.identity.revision,
        root_layer: root_layer.to_owned(),
        schema,
        complete: token.completeness == Completeness::Complete,
        structure_digest: token.structure_digest,
        prim_count: token.prim_count as u64,
    }
}

fn reject(error: impl Into<PlateError>) -> ContentRejection {
    let error: PlateError = error.into();
    ContentRejection {
        code: error.code,
        message: error.message,
    }
}

/// Opens exactly the committed bytes in a fresh reader and derives everything from them, so the
/// index and the verified plates cannot be stale, partial or foreign.
struct PlateVerifier;

impl ContentVerifier for PlateVerifier {
    fn verify(&self, request: &VerifyRequest<'_>) -> Result<IndexEvidence, ContentRejection> {
        let assets = request
            .layers
            .iter()
            .map(|l| Asset {
                name: l.name.into(),
                digest: l.digest,
                bytes: l.bytes.to_vec(),
            })
            .collect();
        let pinned = match request.pinned_revision {
            Some(base) => base,
            None => initial_pin().map_err(reject)?,
        };
        let external = request
            .layers
            .iter()
            .filter(|l| l.role == LayerRole::External)
            .map(|l| l.name.to_owned())
            .collect();
        let open = open_request(
            request.root_layer,
            request.project.as_str(),
            request.workspace.as_str(),
            pinned,
            request.generation,
            assets,
            &external,
        )
        .map_err(reject)?;
        let control = control();
        let schema = schema_identity().map_err(reject)?;
        if schema.digest != request.schema {
            return Err(reject(integrity(
                "draft schema is not the admitted U02 schema",
            )));
        }
        validate_layers(open.clone(), schema, &control).map_err(reject)?;
        let mut reader = Reader::default();
        let opened = reader.open(open, &control).map_err(reject)?;
        // D1: a recorded ProjectId is the revision's own, so a stored record can never disagree
        // with the project of the manifest, nor change between revisions of one project.
        if let Some(recorded) = project_record(&opened.metadata).map_err(reject)?
            && recorded != request.project.as_str()
        {
            return Err(reject(integrity(
                "the project record differs from the revision's project",
            )));
        }
        let index = reader
            .element_index(&opened.context, &control)
            .map_err(reject)?;
        reader.verify_index(&index, &control).map_err(reject)?;
        let decoded = decode(&reader, &opened, &control).map_err(reject)?;
        let (records, member_records) = evaluate_decoded(&decoded).map_err(reject)?;
        verify_derived(&records, &member_records, &decoded, true).map_err(reject)?;
        Ok(evidence(&index, request.root_layer, request.schema))
    }
}

/// One layer of a revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerSummary {
    /// `authored`, `derived` or `external`.
    pub role: &'static str,
    /// Virtual layer name.
    pub name: String,
    /// Full SHA-256 of the layer bytes.
    pub digest: ContentDigest,
    /// Layer size in bytes.
    pub size: u64,
}

/// What the host recorded about a revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionSummary {
    /// Full revision UUIDv7.
    pub revision_id: String,
    /// Head-log sequence.
    pub sequence: u64,
    /// SHA-256 of the manifest blob: the content identity of the revision.
    pub manifest_digest: ContentDigest,
    /// Project, workspace, operation and generation, when the manifest is available.
    pub details: Option<RevisionDetails>,
}

/// Manifest fields of a revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionDetails {
    /// Project UUIDv7.
    pub project: String,
    /// Workspace UUIDv7.
    pub workspace: String,
    /// Operation that created the revision.
    pub op: String,
    /// Workspace generation.
    pub generation: u64,
    /// Layers in manifest order.
    pub layers: Vec<LayerSummary>,
    /// U03 index structure digest and prim count recorded at commit.
    pub index: Option<(ContentDigest, u64)>,
}

pub(crate) fn summarize(info: &RevisionInfo) -> RevisionSummary {
    let manifest = info.manifest();
    let content = manifest.content();
    RevisionSummary {
        revision_id: info.revision().as_str().to_owned(),
        sequence: info.sequence(),
        manifest_digest: info.manifest_digest(),
        details: Some(RevisionDetails {
            project: content.project().as_str().to_owned(),
            workspace: content.workspace().as_str().to_owned(),
            op: content.op().as_str().to_owned(),
            generation: content.workspace_generation(),
            layers: content
                .entries()
                .iter()
                .map(|e| LayerSummary {
                    role: e.role().as_str(),
                    name: e.name().to_owned(),
                    digest: e.digest(),
                    size: e.size(),
                })
                .collect(),
            index: manifest.index().map(|i| (i.structure, i.prims)),
        }),
    }
}

/// Inputs of one `create_plate` call beyond the command itself.
#[derive(Debug, Clone)]
pub struct CreateRequest {
    /// The logical operation. A retry with the same id and identical content is a replay.
    pub op_id: OpId,
    /// Identity of the revision this call creates. Only used when the call creates one.
    pub revision_id: RevisionId,
    /// The creating actor (trusted context, `bim:createdBy`).
    pub actor: Uuid7,
    /// Project of a store that has no revision yet. Ignored otherwise: the store's wins.
    pub project: ProjectId,
    /// Workspace of a store that has no revision yet. Ignored otherwise.
    pub workspace: WorkspaceId,
    /// The command.
    pub command: CreatePlate,
}

/// Result of `create_plate`.
#[derive(Debug, Clone, PartialEq)]
pub struct Created {
    /// True when the same operation was already committed and nothing was written.
    pub replayed: bool,
    /// The element the command created (or created earlier).
    pub plate: PlateRecord,
    /// The revision that holds it.
    pub revision: RevisionSummary,
    /// The footprint the engine declared; `None` for a replay, which applies nothing.
    pub footprint: Option<Footprint>,
}

/// Revision id, sequence and manifest digest the host ledger recorded for an operation.
pub(crate) type Recorded = (String, u64, ContentDigest);

fn prior_op(
    store: &RevisionStore,
    op: &Uuid7,
    command: &CreatePlate,
    head_plates: &[PlateRecord],
) -> Result<(PriorOp, Option<Recorded>), PlateError> {
    prior_op_with(store, op, || {
        // The recorded request is reconstructed from the element of that id in the head
        // model; an absent element can never match, so the command is reuse.
        match head_plates.iter().find(|r| r.authored.id == command.new_id) {
            Some(record) => Ok(record.authored.to_command().content_fingerprint()?),
            None => digest_of(b"no accepted request with this element id"),
        }
    })
}

/// What the host ledger proved about an operation id. `fingerprint` reconstructs the content
/// fingerprint of the accepted request from the head model and is only called for a committed
/// operation.
pub(crate) fn prior_op_with(
    store: &RevisionStore,
    op: &Uuid7,
    fingerprint: impl FnOnce() -> Result<ContentDigest, PlateError>,
) -> Result<(PriorOp, Option<Recorded>), PlateError> {
    match store.query_outcome(op)? {
        Outcome::Committed {
            revision,
            sequence,
            manifest,
            ..
        } => {
            let fingerprint = fingerprint()?;
            Ok((
                PriorOp::Accepted { fingerprint },
                Some((revision.as_str().to_owned(), sequence, manifest)),
            ))
        }
        Outcome::Aborted { .. } | Outcome::NotFound => Ok((PriorOp::Unseen, None)),
        Outcome::Unknown => Err(PlateError::new(
            ErrorCode::OutcomeUnknown,
            "the operation id may or may not have been committed; run revision-recover",
        )),
    }
}

/// The definitions an element was bound to by an operation that the host ledger accepted.
#[derive(Debug, Clone)]
pub struct AcceptedBinding {
    /// The profile the element pins (members only).
    pub profile: Option<ContentDigest>,
    /// The material the element pins, if any.
    pub material: Option<ContentDigest>,
    /// The definitions of the head revision, to read the names of those digests.
    pub definitions: BTreeMap<ContentDigest, DefinitionRecord>,
}

/// What an already accepted operation bound its element to, so that a retry by name replays the
/// original binding instead of resolving the name again (a definition published since may make
/// the name ambiguous). `None` when the ledger has not committed the operation or the head has no
/// plate or member with this id; the caller then resolves names normally and the engine's
/// content fingerprint decides replay or `OP_ID_REUSE`. An uncertain outcome is
/// `OUTCOME_UNKNOWN`, as for admission.
pub fn accepted_binding(
    store: &RevisionStore,
    op_id: &OpId,
    element: &ElementId,
) -> Result<Option<AcceptedBinding>, PlateError> {
    match store.query_outcome(&host_uuid(&op_id.to_string())?)? {
        Outcome::Committed { .. } => {}
        Outcome::Aborted { .. } | Outcome::NotFound => return Ok(None),
        Outcome::Unknown => {
            return Err(PlateError::new(
                ErrorCode::OutcomeUnknown,
                "the operation id may or may not have been committed; run revision-recover",
            ));
        }
    }
    let revision = match store.open() {
        Ok(revision) => revision,
        Err(RevisionError::NoRevision) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let control = control();
    let request = open_base(&base_of(&revision)?)?;
    let mut reader = Reader::default();
    let opened = reader.open(request, &control)?;
    let decoded = decode(&reader, &opened, &control)?;
    let bound = decoded
        .plates
        .iter()
        .find(|(p, _)| p.id == *element)
        .map(|(p, _)| (None, p.material))
        .or_else(|| {
            decoded
                .members
                .iter()
                .find(|(m, _)| m.id == *element)
                .map(|(m, _)| (Some(m.profile), m.material))
        });
    Ok(bound.map(|(profile, material)| AcceptedBinding {
        profile,
        material,
        definitions: decoded.definitions,
    }))
}

/// The revision a replayed operation reports: the head when it is that revision, otherwise the
/// summary the ledger recorded.
pub(crate) fn replay_summary(
    head: &Option<Revision>,
    recorded: Option<Recorded>,
) -> Result<RevisionSummary, PlateError> {
    match (head, recorded) {
        (Some(revision), Some((id, _, _))) if revision.info().revision().as_str() == id => {
            Ok(summarize(revision.info()))
        }
        (_, Some((revision_id, sequence, manifest_digest))) => Ok(RevisionSummary {
            revision_id,
            sequence,
            manifest_digest,
            details: None,
        }),
        _ => Err(integrity("replay without a recorded revision")),
    }
}

/// The U02 mesh change of a kernel body.
pub(crate) fn mesh_change(
    id: &ElementId,
    solid: &SolidMesh,
    record: &QuantityRecord,
) -> Result<Change, PlateError> {
    let mesh = kernel_mesh(solid)?;
    Ok(Change::Mesh(beamcraft_layer_engine::authored::Mesh {
        id: id.to_string(),
        shape_key: solid.digest(),
        quantities: Some(stored_quantities(record)),
        points_mm: mesh.points.clone(),
        face_counts: vec![3; mesh.counts.len()],
        indices: mesh
            .indices
            .iter()
            .map(|i| i32::try_from(*i))
            .collect::<Result<_, _>>()
            .map_err(|_| integrity("mesh index does not fit"))?,
    }))
}

/// The layer engine's own write set must lie inside the footprint the engine declared.
pub(crate) fn check_footprint(prepared: &Prepared, declared: &Footprint) -> Result<(), PlateError> {
    let observed: BTreeSet<Entry> = prepared
        .writes
        .iter()
        .map(layer_entry)
        .collect::<Option<_>>()
        .ok_or_else(|| integrity("layer engine footprint names an unknown target"))?;
    if !declared.covers_writes(&observed) {
        return Err(integrity(
            "layer engine writes more than the engine footprint declares",
        ));
    }
    Ok(())
}

fn layer_group(group: LayerGroup) -> Group {
    match group {
        LayerGroup::Lifecycle => Group::Lifecycle,
        LayerGroup::Label => Group::Label,
        LayerGroup::ShapeInputs => Group::ShapeInputs,
        LayerGroup::Placement => Group::Placement,
        LayerGroup::Material => Group::MaterialRef,
        LayerGroup::Derived => Group::Derived,
        LayerGroup::Relationship => Group::Relationship,
        LayerGroup::Mark => Group::Mark,
    }
}

fn layer_entry(item: &beamcraft_layer_engine::authored::Footprint) -> Option<Entry> {
    let target = match ElementId::parse(&item.identity) {
        Ok(id) => Target::Element(id),
        Err(_) => Target::Definition(item.identity.parse().ok()?),
    };
    Some(Entry {
        target,
        group: layer_group(item.group),
    })
}

/// The layers a new revision is authored over: the verified head, or the empty initial model.
pub(crate) struct Base {
    pub(crate) project: String,
    pub(crate) workspace: String,
    pub(crate) digest: ContentDigest,
    pub(crate) generation: u64,
    pub(crate) assets: Vec<Asset>,
    /// Names of the layers that are foreign sources: stored and composed, never owned or written.
    pub(crate) external: BTreeSet<String>,
    pub(crate) expected: ExpectedBase,
}

/// The layers of one stored revision as a base to open or author over.
pub(crate) fn base_of(revision: &Revision) -> Result<Base, PlateError> {
    let content = revision.manifest().content();
    let assets = content
        .entries()
        .iter()
        .map(|e| {
            Ok(Asset {
                name: e.name().into(),
                digest: e.digest(),
                bytes: revision.layer_bytes(e.name())?,
            })
        })
        .collect::<Result<Vec<_>, PlateError>>()?;
    Ok(Base {
        project: content.project().as_str().to_owned(),
        workspace: content.workspace().as_str().to_owned(),
        digest: revision.info().manifest_digest(),
        generation: content.workspace_generation(),
        assets,
        external: content
            .entries()
            .iter()
            .filter(|e| e.role() == LayerRole::External)
            .map(|e| e.name().to_owned())
            .collect(),
        expected: ExpectedBase::Revision(revision.info().revision().clone()),
    })
}

/// Opens the head of the store (`None` for a store without a revision) and the base that the next
/// revision builds on. A store without a revision takes the given identities and records the
/// project (D1); a store with one keeps its own, unless the caller holds a [`ProjectClaim`]: the
/// claim is compared with the project of the exact head opened here, which is also the base the
/// revision is committed on, so a store initialised by another writer after an earlier check can
/// never adopt a claimed project silently. A first revision records the claimed project.
pub(crate) fn load_base(
    store: &RevisionStore,
    project: &ProjectId,
    workspace: &WorkspaceId,
) -> Result<(Option<Revision>, Base), PlateError> {
    let claim = CLAIMED_PROJECT.with(Cell::get);
    let head = match store.open() {
        Ok(revision) => Some(revision),
        Err(RevisionError::NoRevision) => None,
        Err(error) => return Err(error.into()),
    };
    let base = match &head {
        Some(revision) => {
            let base = base_of(revision)?;
            if claim.is_some_and(|c| c.to_string() != base.project) {
                return Err(project_mismatch());
            }
            base
        }
        None => {
            let project = claim.as_ref().unwrap_or(project);
            Base {
                project: project.to_string(),
                workspace: workspace.to_string(),
                digest: initial_pin()?,
                generation: 0,
                assets: vec![
                    asset(ROOT_LAYER, empty_root(project).as_bytes())?,
                    asset(AUTHORED_LAYER, EMPTY_LAYER.as_bytes())?,
                    asset(DERIVED_LAYER, EMPTY_LAYER.as_bytes())?,
                ],
                external: BTreeSet::new(),
                expected: ExpectedBase::Initial,
            }
        }
    };
    Ok((head, base))
}

fn project_mismatch() -> PlateError {
    PlateError::new(
        ErrorCode::InvalidArgument,
        "the project id differs from the project recorded in the store",
    )
}

thread_local! {
    static CLAIMED_PROJECT: Cell<Option<ProjectId>> = const { Cell::new(None) };
}

/// D1: an explicit project claim of the current thread, held until dropped. While it is held every
/// revision this thread authors is built on a base of that project: `load_base` refuses a head
/// of another project with `INVALID_ARGUMENT` and a first revision records the claimed one. The
/// CLI holds one per command that names `--project-id`; a library caller that names a project
/// holds one too. Claims nest and restore the previous one.
#[must_use = "the claim ends when it is dropped"]
pub struct ProjectClaim {
    previous: Option<ProjectId>,
}

/// Starts a [`ProjectClaim`] on the current thread.
pub fn claim_project(project: ProjectId) -> ProjectClaim {
    ProjectClaim {
        previous: CLAIMED_PROJECT.with(|c| c.replace(Some(project))),
    }
}

impl Drop for ProjectClaim {
    fn drop(&mut self) {
        CLAIMED_PROJECT.with(|c| c.set(self.previous));
    }
}

/// D1: an early, typed check of a project id that the caller names explicitly: a store without a
/// revision accepts any (the first revision records it); a store with one refuses a different id
/// with `INVALID_ARGUMENT`. It is advisory: the binding check is the [`ProjectClaim`] that
/// `load_base` compares with the base it builds on. Callers that give no id take the store's.
pub fn ensure_project(store: &RevisionStore, claimed: &ProjectId) -> Result<(), PlateError> {
    match store.open() {
        Ok(head) if head.manifest().content().project().as_str() == claimed.to_string() => Ok(()),
        Ok(_) => Err(project_mismatch()),
        Err(RevisionError::NoRevision) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// The pinned stage of a base, opened for reading.
pub(crate) fn open_base(base: &Base) -> Result<OpenRequest, PlateError> {
    open_request(
        ROOT_LAYER,
        &base.project,
        &base.workspace,
        base.digest,
        base.generation,
        base.assets.clone(),
        &base.external,
    )
}

/// Creates one plate and publishes it as the next revision of the store, or replays it.
///
/// Steps, each failing closed: open and verify the head; decide replay from the host ledger;
/// engine admission (`CreatePlate` validation, a pinned material resolved from the definitions
/// of the head, kernel body, footprint); U02 authoring of the
/// model and derived layers over the head's layers; check that the layer engine's own write set
/// is inside the engine's declared footprint; publish layer blobs; commit through U06/U07 with
/// the verifier above (U03 index, structural validation, kernel re-evaluation of every plate);
/// `hook` observes the commit points for fault injection (pass `&mut NoFault` in production).
pub fn create_plate(
    store: &RevisionStore,
    request: &CreateRequest,
    hook: &mut dyn CommitHook,
) -> Result<Created, PlateError> {
    let control = control();
    let (head, base) = load_base(store, &request.project, &request.workspace)?;
    let base_request = open_base(&base)?;
    let Base {
        project,
        workspace,
        generation: base_generation,
        expected,
        ..
    } = base;
    let (head_records, head_members, definitions) = {
        let mut reader = Reader::default();
        let opened = reader.open(base_request.clone(), &control)?;
        let decoded = decode(&reader, &opened, &control)?;
        let (records, members) = evaluate_decoded(&decoded)?;
        (records, members, decoded.definitions)
    };
    let op_host = host_uuid(&request.op_id.to_string())?;
    let (prior, recorded) = prior_op(store, &op_host, &request.command, &head_records)?;
    let mut existing = ids(&head_records);
    existing.extend(head_members.iter().map(|m| m.authored.id));
    let admission = admit_create_plate(
        request.op_id,
        &request.command,
        prior,
        &|id: &ElementId| existing.contains(id),
        &material_lookup(&definitions),
    )?;
    let transaction = match admission {
        Admission::Replay => {
            let record = head_records
                .into_iter()
                .find(|r| r.authored.id == request.command.new_id)
                .ok_or_else(|| integrity("replayed element is absent from the head revision"))?;
            return Ok(Created {
                replayed: true,
                plate: record,
                revision: replay_summary(&head, recorded)?,
                footprint: None,
            });
        }
        Admission::Apply(transaction) => *transaction,
    };
    let outcome = &transaction.outcome;
    let record = PlateRecord {
        authored: outcome.authored.clone(),
        body: outcome.body.clone(),
        quantities: outcome.quantities,
    };
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
    let id_text = outcome.authored.id.to_string();
    let placement = outcome.authored.placement;
    let [qx, qy, qz, qw] = placement.orientation_xyzw;
    let prepared = author.prepare(
        Prepare {
            context: opened.context.clone(),
            op_id: request.op_id.to_string(),
            actor: request.actor.to_string(),
            schema,
            resources: Vec::new(),
            changes: vec![
                Change::Create(Element {
                    id: id_text,
                    name: outcome.authored.name.clone(),
                    shape: Shape::Plate {
                        contour_mm: outcome.authored.contour_points_mm.clone(),
                        thickness_mm: outcome.authored.thickness_mm,
                        holes: outcome
                            .authored
                            .holes_mm
                            .iter()
                            .map(|h| h.iter().map(|[x, y]| [*x, *y, 0.0]).collect())
                            .collect(),
                    },
                    placement: LayerPlacement {
                        translation_mm: placement.translation_mm,
                        orientation_xyzw: [qx, qy, qz, qw],
                    },
                    material: outcome.authored.material,
                }),
                mesh_change(
                    &outcome.authored.id,
                    &record.body.solid,
                    &plate_record(&record),
                )?,
            ],
        },
        &control,
    )?;
    check_footprint(&prepared, &outcome.footprint)?;
    let committed = publish_revision(
        store,
        &PublishRequest {
            prepared: &prepared,
            schema,
            project: &project,
            workspace: &workspace,
            revision_id: &request.revision_id,
            op: op_host,
            base_generation,
            expected: &expected,
        },
        hook,
    )?;
    Ok(Created {
        replayed: committed.outcome == CommitOutcome::Replayed,
        plate: record,
        revision: summarize(&committed.info),
        footprint: Some(transaction.outcome.footprint.clone()),
    })
}

/// What `publish_revision` needs to turn a prepared candidate into a committed revision.
pub(crate) struct PublishRequest<'a> {
    pub(crate) prepared: &'a Prepared,
    pub(crate) schema: SchemaIdentity,
    pub(crate) project: &'a str,
    pub(crate) workspace: &'a str,
    pub(crate) revision_id: &'a RevisionId,
    pub(crate) op: Uuid7,
    pub(crate) base_generation: u64,
    pub(crate) expected: &'a ExpectedBase,
}

/// Stores the layer blobs of a prepared candidate and commits them as the next revision through
/// U06/U07, with the content verifier of this module.
pub(crate) fn publish_revision(
    store: &RevisionStore,
    request: &PublishRequest<'_>,
    hook: &mut dyn CommitHook,
) -> Result<Committed, PlateError> {
    let generation = request
        .base_generation
        .checked_add(1)
        .ok_or(PlateError::new(
            ErrorCode::LimitExceeded,
            "generation exhausted",
        ))?;
    let mut entries = Vec::new();
    for layer in &request.prepared.layers {
        store
            .blobs()
            .publish(&layer.bytes)
            .map_err(|e| PlateError::new(e.code(), "layer blob could not be stored"))?;
        entries.push(LayerEntry::from_bytes(
            &layer.name,
            role_of(&layer.name),
            &layer.bytes,
        )?);
    }
    // Foreign sources are carried into every revision byte for byte as external entries. Their
    // blob is published if absent (publication compares existing bytes in full) and is never
    // rewritten.
    for layer in &request.prepared.retained {
        store
            .blobs()
            .publish(&layer.bytes)
            .map_err(|e| PlateError::new(e.code(), "foreign layer blob could not be stored"))?;
        entries.push(LayerEntry::from_bytes(
            &layer.name,
            LayerRole::External,
            &layer.bytes,
        )?);
    }
    let draft = RevisionDraft {
        project: host_uuid(request.project)?,
        workspace: host_uuid(request.workspace)?,
        revision: host_uuid(&request.revision_id.to_string())?,
        op: request.op.clone(),
        workspace_generation: generation,
        schema: request.schema.digest,
        root_layer: ROOT_LAYER.into(),
        entries,
    };
    Ok(store.commit_with(&draft, request.expected, Some(&PlateVerifier), hook)?)
}

/// Commits a complete layer set through U06/U07 with the content verifier of this module (U03
/// index, structural validation, kernel re-evaluation of every element). Used where the layers
/// already exist as bytes, such as forking and publishing a workspace.
pub(crate) fn commit_verified(
    store: &RevisionStore,
    draft: &RevisionDraft,
    expected: &ExpectedBase,
    hook: &mut dyn CommitHook,
) -> Result<Committed, PlateError> {
    Ok(store.commit_with(draft, expected, Some(&PlateVerifier), hook)?)
}

/// A plate as listed from a committed revision.
#[derive(Debug, Clone)]
pub struct Inspection {
    /// The verified revision.
    pub revision: RevisionSummary,
    /// Committed plates in element-id order, re-evaluated by the engine.
    pub plates: Vec<PlateRecord>,
    /// Committed beams and columns in element-id order, re-evaluated by the engine.
    pub members: Vec<MemberRecord>,
    /// Stored quantity records against the engine's, by element id.
    pub quantities: BTreeMap<ElementId, QuantityReport>,
    /// Elements that are not plates (not created by this slice).
    pub other_elements: usize,
    /// The immutable profile and material definitions of the revision, by full digest.
    pub definitions: BTreeMap<ContentDigest, DefinitionRecord>,
    /// The pinned view snapshot built from committed data.
    pub snapshot: ViewSnapshot,
    /// D1: the ProjectId recorded in the root layer of the revision, or `None` for a store whose
    /// first revision predates the record (never invented for it).
    pub project_record: Option<String>,
    /// D2: the levels of the revision and the level of every plate, beam and column.
    pub levels: Levels,
    /// The common facts of every element except levels, by id (E02).
    pub elements: BTreeMap<ElementId, ElementMeta>,
    /// The marks of the model and their approval states (A08).
    pub marks: Numbering,
}

/// Opens the head revision with verification, reads its model and derived layers in a fresh
/// reader, re-evaluates every plate with the kernel, checks the committed meshes against the
/// kernel's and builds the pinned view snapshot.
pub fn inspect(store: &RevisionStore) -> Result<Inspection, PlateError> {
    inspect_revision(&store.open()?)
}

/// [`inspect`] for one already opened and verified revision (the head or an earlier one).
pub fn inspect_revision(revision: &Revision) -> Result<Inspection, PlateError> {
    let content = revision.manifest().content();
    let assets = content
        .entries()
        .iter()
        .map(|e| {
            Ok(Asset {
                name: e.name().into(),
                digest: e.digest(),
                bytes: revision.layer_bytes(e.name())?,
            })
        })
        .collect::<Result<Vec<_>, PlateError>>()?;
    let external = content
        .entries()
        .iter()
        .filter(|e| e.role() == LayerRole::External)
        .map(|e| e.name().to_owned())
        .collect();
    let request = open_request(
        content.root_layer(),
        content.project().as_str(),
        content.workspace().as_str(),
        revision.info().manifest_digest(),
        content.workspace_generation(),
        assets,
        &external,
    )?;
    let control = control();
    // The structural rules (D2-V01 among them) run before anything is decoded, so a damaged
    // level or project record is refused rather than reported.
    validate_layers(request.clone(), schema_identity()?, &control)?;
    let mut reader = Reader::default();
    let opened = reader.open(request, &control)?;
    let record = project_record(&opened.metadata)?;
    if record
        .as_deref()
        .is_some_and(|recorded| recorded != content.project().as_str())
    {
        return Err(integrity(
            "the project record differs from the revision's project",
        ));
    }
    let index = reader.element_index(&opened.context, &control)?;
    reader.verify_index(&index, &control)?;
    let decoded = decode(&reader, &opened, &control)?;
    let (plates, members) = evaluate_decoded(&decoded)?;
    let quantities = verify_derived(&plates, &members, &decoded, false)?;
    let pin = ViewPin::new(
        revision.info().manifest_digest(),
        content.workspace_generation(),
    );
    let snapshot = ViewSnapshot::build(
        pin,
        scene_elements_with_members(&plates, &members)?,
        &Limits::default(),
    )
    .map_err(|e| PlateError::new(e.f04_code(), "view snapshot could not be built"))?;
    Ok(Inspection {
        revision: summarize(revision.info()),
        plates,
        members,
        quantities,
        other_elements: decoded.other_elements,
        definitions: decoded.definitions,
        snapshot,
        project_record: record,
        levels: decoded.levels,
        elements: decoded.elements,
        marks: decoded.marks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_layer_engine::Property;

    fn prim(properties: Vec<Property>) -> Prim {
        Prim {
            path: "/World/Steel/E_0".into(),
            element_id: None,
            type_name: None,
            active: true,
            kind: None,
            display_name: None,
            properties,
            relationships: Vec::new(),
            sources: Vec::new(),
        }
    }

    fn property(name: &str, value: Option<Value>) -> Property {
        Property {
            name: name.into(),
            type_name: "double3[]".into(),
            value,
            interpolation: None,
        }
    }

    fn mesh(points: usize, counts: Vec<i64>, indices: Vec<i64>) -> DerivedMesh {
        DerivedMesh {
            points: vec![[0.0, 0.0, 0.0]; points],
            counts,
            indices,
            shape_key: String::new(),
            orientation: "rightHanded".into(),
            subdivision: "none".into(),
            stored: None,
        }
    }

    #[test]
    fn only_a_triangle_mesh_with_valid_indices_is_published() {
        let good = published_mesh(&mesh(3, vec![3], vec![0, 1, 2])).unwrap();
        assert_eq!(good.triangles, [[0, 1, 2]]);
        for bad in [
            mesh(0, vec![3], vec![0, 1, 2]),
            mesh(3, vec![], vec![]),
            mesh(4, vec![4], vec![0, 1, 2, 3]),
            mesh(3, vec![3], vec![0, 1, 3]),
            mesh(3, vec![3], vec![0, 1, -1]),
            mesh(3, vec![3], vec![0, 1]),
            mesh(3, vec![3, 3], vec![0, 1, 2]),
        ] {
            assert!(published_mesh(&bad).is_none());
        }
        let mut nan = mesh(3, vec![3], vec![0, 1, 2]);
        nan.points[0][0] = f32::NAN;
        assert!(published_mesh(&nan).is_none());
    }

    #[test]
    fn a_placement_is_rigid_only_in_the_pose_form() {
        let numbers = |name: &str, ty: &str, values: &[f64]| Property {
            name: name.into(),
            type_name: ty.into(),
            value: Some(Value::Array(
                values.iter().map(|v| Value::Number(*v)).collect(),
            )),
            interpolation: None,
        };
        let order = |ops: &[&str]| Property {
            name: "xformOpOrder".into(),
            type_name: "token[]".into(),
            value: Some(Value::Array(
                ops.iter().map(|o| Value::Text((*o).into())).collect(),
            )),
            interpolation: None,
        };
        let translate = numbers("xformOp:translate", "double3", &[1.0, 2.0, 3.0]);
        let orient = numbers("xformOp:orient", "quatd", &[1.0, 0.0, 0.0, 0.0]);
        let pose_order = order(&["xformOp:translate", "xformOp:orient"]);
        assert_eq!(placement_read(&prim(Vec::new())), PlacementRead::Absent);
        let rigid = placement_read(&prim(vec![
            translate.clone(),
            orient.clone(),
            pose_order.clone(),
        ]));
        assert_eq!(
            rigid,
            PlacementRead::Rigid(beamcraft_model::command::Placement {
                translation_mm: [1.0, 2.0, 3.0],
                orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
            })
        );
        // A missing half, another order and an additional operation are not the pose form.
        for properties in [
            vec![translate.clone(), pose_order.clone()],
            vec![
                translate.clone(),
                orient.clone(),
                order(&["xformOp:orient", "xformOp:translate"]),
            ],
            vec![
                translate.clone(),
                orient.clone(),
                pose_order.clone(),
                numbers("xformOp:scale:bimUnits", "double3", &[1.0, 1.0, 1.0]),
            ],
            vec![translate, orient],
        ] {
            assert_eq!(placement_read(&prim(properties)), PlacementRead::Other);
        }
    }

    #[test]
    fn a_hole_declaration_without_a_value_is_a_damaged_record_not_no_holes() {
        assert_eq!(
            decode_holes(&prim(Vec::new())).unwrap(),
            Vec::<Vec<[f64; 2]>>::new()
        );
        let lone = prim(vec![property("steel:plate:holePointsMm", None)]);
        assert!(decode_holes(&lone).is_err());
        let lone_counts = prim(vec![property("steel:plate:holeVertexCounts", None)]);
        assert!(decode_holes(&lone_counts).is_err());
    }
}
