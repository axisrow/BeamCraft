// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A06 adapter: `CreateWeld`, committed as the next revision, plus a read of the welds of the
//! head.
//!
//! A weld is an element of kind `connection` with its own UUIDv7, label and identity placement,
//! explicitly authored weld properties (type, size, workshop or site, side) and one
//! `bim:connects` record naming the main part, then the part welded to it. It has no material and
//! no derived body: no solid weld geometry is promised. The record lives with the weld, so the
//! parts are only read. The engine ([`beamcraft_engine::weld`]) decides the identity and the
//! parts' kinds from the head model; the layer engine re-checks the same rules on the candidate
//! and the strict verifier on the committed bytes.
//!
//! Replay of an `op_id` is decided from the revision the host ledger recorded for it: the
//! operation is a replay only when that revision created exactly this weld (present there, absent
//! from the revision before), whatever the head holds now. No command renames, edits or
//! moves a weld yet; its placement is the identity. Deleting a welded part is `DEPENDENCY_CONFLICT` while the weld connects it (no cascade); deleting the weld
//! is `delete-element`. Copying a weld is refused (`DV-REL-011`).

use crate::assembly::read_prims;
use crate::plate::{
    AUTHORED_LAYER, DERIVED_LAYER, PlateError, RevisionSummary, base_of, check_footprint, control,
    digest_of, host_uuid, integrity, load_base, mesh_change, number, numbers, open_base,
    prior_op_with, property, replay_summary, summarize, text,
};
use crate::publication::{PublishRequest, publish_revision};
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::quantity::QuantityRecord;
use beamcraft_engine::transaction::PriorOp;
use beamcraft_engine::weld::{PartKind, Parts};
use beamcraft_host::revision::{CommitHook, CommitOutcome, RevisionError, RevisionStore, Uuid7};
use beamcraft_layer_engine::authored::{
    Author, Change, EditTargets, Element, Placement as LayerPlacement, Prepare, Shape, Weld,
    WeldPath, schema_identity, validate_layers,
};
use beamcraft_layer_engine::{Control, OpenRequest, Prim, Reader, Value};
use beamcraft_model::command::{
    CommandError, CreateWeld, WeldContour, WeldGeometry, WeldLocation, WeldSide, WeldSpec,
    WeldSymbol, WeldType,
};
use beamcraft_model::digest::Sha256;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId};
use std::collections::BTreeMap;

/// Identities of one A06 call beyond the command itself.
#[derive(Debug, Clone)]
pub struct WeldContext {
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

/// One weld of the head.
#[derive(Debug, Clone, PartialEq)]
pub struct WeldView {
    /// Identity.
    pub id: ElementId,
    /// Label.
    pub name: String,
    /// The main part.
    pub main: ElementId,
    /// The part welded to it.
    pub secondary: ElementId,
    /// The authored properties.
    pub spec: WeldSpec,
    /// The authored geometry and the stored solid facts (A15); `None` for an A06 weld without
    /// a path.
    pub solid: Option<WeldSolidView>,
}

/// The path, symbol data and stored volume of one geometric weld (A15).
#[derive(Debug, Clone, PartialEq)]
pub struct WeldSolidView {
    /// The authored path and symbol data.
    pub geometry: WeldGeometry,
    /// World length of the path in millimetres.
    pub length_mm: f64,
    /// The stored analytic volume of the weld solid in mm3.
    pub volume_mm3: f64,
}

/// Result of `create_weld`.
#[derive(Debug, Clone, PartialEq)]
pub struct WeldChange {
    /// True when the same operation was already committed and nothing was written.
    pub replayed: bool,
    /// The revision that holds the result.
    pub revision: RevisionSummary,
    /// The declared footprint; `None` for a replay.
    pub footprint: Option<Footprint>,
    /// Names of the layers whose bytes changed (empty for a replay).
    pub changed_layers: Vec<String>,
    /// The welds of the resulting head.
    pub welds: Vec<WeldView>,
}

/// The kinds of a pinned model's elements and its welds.
struct Model {
    parts: Parts,
    welds: Vec<WeldView>,
}

fn weld_of(
    prim: &Prim,
    id: ElementId,
    by_path: &BTreeMap<&str, ElementId>,
    prims: &[Prim],
) -> Result<WeldView, PlateError> {
    let damaged = || integrity("weld record is not readable");
    let token = |name| property(prim, name).and_then(text).ok_or_else(damaged);
    let spec = WeldSpec {
        weld_type: WeldType::parse(token("steel:weld:type")?).ok_or_else(damaged)?,
        size_mm: property(prim, "steel:weld:sizeMm")
            .and_then(number)
            .ok_or_else(damaged)?,
        location: WeldLocation::parse(token("steel:weld:location")?).ok_or_else(damaged)?,
        side: WeldSide::parse(token("steel:weld:side")?).ok_or_else(damaged)?,
    };
    // The optional A15 geometry: the path, the symbol flag and token, and the stored volume of
    // the derived solid. An absent symbol attribute reads as its authored absence (unset flag,
    // no contour symbol); a path weld without a stored volume is a damaged record.
    let solid = match property(prim, "steel:weld:pathMm") {
        None => None,
        Some(Value::Array(items)) => {
            let path_mm = items
                .iter()
                .map(numbers::<3>)
                .collect::<Option<Vec<_>>>()
                .ok_or_else(damaged)?;
            let all_around = matches!(
                property(prim, "steel:weld:allAround").and_then(text),
                Some("true")
            );
            let contour = WeldContour::parse(
                property(prim, "steel:weld:contour")
                    .and_then(text)
                    .unwrap_or("none"),
            )
            .ok_or_else(damaged)?;
            let geometry_path = format!("{}/Geometry", prim.path);
            let volume_mm3 = prims
                .iter()
                .find(|p| p.path == geometry_path)
                .and_then(|g| property(g, "bim:volumeMm3"))
                .and_then(number)
                .ok_or_else(damaged)?;
            Some(WeldSolidView {
                length_mm: beamcraft_engine::weld::path_length(&path_mm),
                volume_mm3,
                geometry: WeldGeometry {
                    path_mm,
                    symbol: WeldSymbol {
                        all_around,
                        contour,
                    },
                },
            })
        }
        Some(_) => return Err(damaged()),
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
    Ok(WeldView {
        id,
        name: token("bim:name")?.to_owned(),
        main: part(main)?,
        secondary: part(secondary)?,
        spec,
        solid,
    })
}

fn model_of(request: &OpenRequest, control: &Control) -> Result<Model, PlateError> {
    // The structural rules (A06-V01 among them) run before the welds are read, so a damaged
    // record is refused rather than reported or replayed.
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
        welds: Vec::new(),
    };
    for prim in &prims {
        let Some(id) = by_path.get(prim.path.as_str()).copied() else {
            continue;
        };
        // Only owned 0.1.0 parametric elements are interpreted, as the structural validator does;
        // newer, foreign or other-mode content stays inert and is never decoded as a weld.
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
                if property(prim, "steel:connection:type").and_then(text) != Some("weld") =>
            {
                PartKind::Other
            }
            Some("connection") => {
                model.welds.push(weld_of(prim, id, &by_path, &prims)?);
                PartKind::Other
            }
            _ => PartKind::Other,
        };
        model.parts.insert(id, kind);
    }
    model.welds.sort_by_key(|w| w.id);
    Ok(model)
}

/// The weld `id` as the operation committed at `sequence` created it: present in that revision
/// and absent (as any element) from the revision before it. `None` when that revision did not
/// create such a weld, so the operation id belongs to another request.
fn created_by(
    store: &RevisionStore,
    sequence: u64,
    id: &ElementId,
    control: &Control,
) -> Result<Option<WeldView>, PlateError> {
    let at = store.open_at(sequence)?;
    let model = model_of(&open_base(&base_of(&at)?)?, control)?;
    let Some(weld) = model.welds.into_iter().find(|w| w.id == *id) else {
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
    Ok(Some(weld))
}

/// Whether the pinned model holds a weld of this identity (for `delete-element`).
pub(crate) fn is_weld(
    request: &OpenRequest,
    id: &ElementId,
    control: &Control,
) -> Result<bool, PlateError> {
    Ok(model_of(request, control)?
        .welds
        .iter()
        .any(|w| w.id == *id))
}

/// The quantity record of a weld solid (A15): the analytic volume with its shape signature; a
/// weld has no material, so its mass is unknown and that absence is part of the mass inputs.
fn weld_record(solid: &beamcraft_engine::weld::WeldSolid) -> Result<QuantityRecord, PlateError> {
    let damaged = || integrity("the weld solid signature could not be extended");
    let shape = solid.shape_signature();
    let mass = |tag: &[u8]| {
        let mut bytes = tag.to_vec();
        bytes.extend_from_slice(shape.as_bytes());
        Sha256::digest(&bytes).map_err(|_| damaged())
    };
    let volume = solid.volume_mm3();
    Ok(QuantityRecord {
        volume_mm3: volume,
        volume_gross_mm3: volume,
        removed_volume_mm3: 0.0,
        volume_status: "current",
        mass_kg: None,
        mass_gross_kg: None,
        mass_status: "unknown",
        shape_signature: shape,
        gross_signature: shape,
        mass_signature: mass(b"BeamCraft/A15/WeldMass/v1\0")?,
        mass_gross_signature: mass(b"BeamCraft/A15/WeldMass/v1\0")?,
        algorithm: "weld-fillet-prism-v1".into(),
        section_area_mm2: None,
        area_source: None,
    })
}

/// The welds of the verified head (empty for a store without a revision).
pub fn welds(store: &RevisionStore) -> Result<Vec<WeldView>, PlateError> {
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
    Ok(model_of(&open_base(&base)?, &control)?.welds)
}

/// Creates one weld between two existing parts and publishes the next revision, or replays the
/// operation.
///
/// Steps, each failing closed: open and verify the head; decide replay from the host ledger and
/// the head state; decide the identity and the parts' kinds with the engine; author with U02;
/// check the layer engine's writes against the declared footprint; commit through U06/U07 with
/// the strict verifier.
pub fn create_weld(
    store: &RevisionStore,
    context: &WeldContext,
    command: &CreateWeld,
    hook: &mut dyn CommitHook,
) -> Result<WeldChange, PlateError> {
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
    // revision, never from the head, so another weld that happens to match is not a replay.
    let (prior, recorded) = prior_op_with(store, &op_host, || {
        digest_of(b"the accepted request is decided from the operation's revision")
    })?;
    let accepted = match (&prior, &recorded) {
        (PriorOp::Accepted { .. }, Some((_, sequence, _))) => {
            created_by(store, *sequence, &command.new_id, &control)?
        }
        _ => None,
    };
    let same = accepted.as_ref().is_some_and(|w| {
        w.name == command.name
            && w.main == command.main_id
            && w.secondary == command.secondary_id
            && w.spec == command.weld
            && w.solid.as_ref().map(|s| &s.geometry) == command.geometry.as_ref()
    });
    match prior {
        PriorOp::Accepted { .. } if same => {
            return Ok(WeldChange {
                replayed: true,
                revision: replay_summary(&head, recorded)?,
                footprint: None,
                changed_layers: Vec::new(),
                welds: model.welds,
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
    let footprint = model.parts.create(command)?;
    // The optional A15 solid: built by the engine from the same authored inputs, so a segment
    // the kernel refuses refuses the command before anything is written.
    let solid = match &command.geometry {
        Some(geometry) => Some(beamcraft_engine::weld::solid(
            command.weld.size_mm,
            geometry,
        )?),
        None => None,
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
    let mut changes = vec![Change::Create(Element {
        id: command.new_id.to_string(),
        name: command.name.clone(),
        shape: Shape::Weld(Weld {
            connects: [
                command.main_id.to_string(),
                command.secondary_id.to_string(),
            ],
            weld_type: command.weld.weld_type.as_token().into(),
            size_mm: command.weld.size_mm,
            location: command.weld.location.as_token().into(),
            side: command.weld.side.as_token().into(),
            geometry: command.geometry.as_ref().map(|g| WeldPath {
                points_mm: g.path_mm.clone(),
                all_around: g.symbol.all_around,
                contour: g.symbol.contour.as_token().into(),
            }),
        }),
        placement: LayerPlacement {
            translation_mm: [0.0; 3],
            orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
        },
        material: None,
    })];
    if let Some(solid) = &solid {
        changes.push(mesh_change(
            &command.new_id,
            solid.mesh(),
            &weld_record(solid)?,
        )?);
    }
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
    Ok(WeldChange {
        replayed: false,
        revision: summarize(&committed.info),
        footprint: Some(footprint),
        changed_layers,
        welds: welds(store)?,
    })
}
