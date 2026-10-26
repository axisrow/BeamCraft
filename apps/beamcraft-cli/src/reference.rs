// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! R01 adapter: attach a foreign USD file as a read-only reference, move the wrapper, state its
//! units explicitly, inspect it, and refuse every write aimed at foreign content.
//!
//! The engine (`beamcraft_engine::reference`) decides; this module is the wiring that only the
//! application layer may do, because it links the file system, the layer engine and the host:
//!
//! ```text
//! attach:  path text --check (no I/O)--> bounded read of the file --layer engine (inspect)-->
//!          pin (digest, size, format, default prim, declared units) --engine--> admission and
//!          footprint --layer engine (U02)--> wrapper in the authored layer, the foreign bytes
//!          carried as a retained external layer --host (U06/U07)--> blobs, verifier, root switch
//! move / correct / inspect / write: the verified head, the wrapper, the same path back
//! ```
//!
//! # What is guaranteed
//!
//! * The text of the source path is classified before the file system, a resolver or the store
//!   is touched; a URL scheme or a network path never reaches `open`.
//! * The source file is only ever opened for reading. Its bytes are stored under their SHA-256 as
//!   an `external` layer of the revision and carried into every later revision unchanged; the
//!   host re-verifies the digest and size on every open, and the verifier refuses a revision in
//!   which any owned layer holds an opinion on a foreign prim.
//! * Placement and units live on the owned wrapper, never on the foreign content.
//!
//! # Limits
//!
//! The foreign layer must be one self-contained USDA or restricted USDC layer within the U01
//! stage budgets (65536 bytes, 256 prims in total); its own sublayers, payloads, variants and
//! network references are refused. Replay of an `op_id` is decided from the head state, as in
//! the other commands. One implicit workspace, in-process layer engine, as in `plate`.

use crate::plate::{
    AUTHORED_LAYER, Base, DERIVED_LAYER, PlateError, RevisionSummary, check_footprint, control,
    digest_of, host_uuid, integrity, load_base, open_base, prior_op_with, replay_summary,
    summarize,
};
use crate::publication::{PublishRequest, publish_revision};
use crate::source_file::read_file;
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::measure::{ReferenceMeasurement, measure_reference};
use beamcraft_engine::reference::{
    ReferenceRecord, UnitsStatus, attach, correct_units, move_reference, write_foreign,
};
use beamcraft_engine::transaction::PriorOp;
use beamcraft_host::revision::{
    CommitHook, CommitOutcome, LayerRole, Revision, RevisionStore, Uuid7,
};
use beamcraft_layer_engine::authored::{
    Author, Change, EditTargets, ForeignPrim, Placement as LayerPlacement, Prepare, Reference,
    ReferenceView, SourcePin, UpAxis, read_references, schema_identity, validate_layers,
};
use beamcraft_layer_engine::{Asset, Budgets, Opened, Prim, Reader, inspect_foreign_layer};
use beamcraft_model::command::{CommandError, Placement};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId};
use beamcraft_model::reference::{
    AttachReference, CorrectReferenceUnits, ForeignTarget, MAX_SOURCE_BYTES, MoveReference,
    WriteForeign, check_source_path,
};
use std::collections::BTreeSet;
use std::convert::Infallible;

/// The identities and trusted context every mutating call carries.
#[derive(Debug, Clone)]
pub struct Ids {
    /// The logical operation. A retry with the same id and identical content is a replay.
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

/// Inputs of one `attach_reference` call.
#[derive(Debug, Clone)]
pub struct AttachRequest {
    /// Identities and context.
    pub ids: Ids,
    /// Identity of the new wrapper.
    pub new_id: ElementId,
    /// Label of the wrapper; the source's file name when `None`.
    pub name: Option<String>,
    /// Rigid placement of the wrapper.
    pub placement: Placement,
    /// The path text of the source file as the user wrote it. It is classified before any I/O and
    /// is never stored.
    pub source_path: String,
}

/// A wrapper as committed, with what the engine says about its units.
#[derive(Debug, Clone, PartialEq)]
pub struct Wrapper {
    /// The wrapper data.
    pub record: ReferenceRecord,
    /// See [`UnitsStatus`].
    pub status: UnitsStatus,
    /// Virtual name of the stored source layer in the revision (`external/<sha256>.usda`).
    pub layer: String,
    /// The foreign prims composed below the wrapper, in the source layer's own namespace.
    pub foreign: Vec<ForeignPrim>,
}

/// Result of `attach_reference`.
#[derive(Debug, Clone, PartialEq)]
pub struct Attached {
    /// True when the same operation was already committed and nothing was written.
    pub replayed: bool,
    /// The wrapper.
    pub wrapper: Wrapper,
    /// True when the head already stored these exact source bytes (another wrapper uses them).
    pub source_reused: bool,
    /// The revision that holds the wrapper.
    pub revision: RevisionSummary,
    /// The declared footprint; `None` for a replay.
    pub footprint: Option<Footprint>,
}

/// Result of `move_reference` and `correct_units`.
#[derive(Debug, Clone, PartialEq)]
pub struct Edited {
    /// True when the same operation was already committed and nothing was written.
    pub replayed: bool,
    /// The wrapper after the edit.
    pub wrapper: Wrapper,
    /// Names of the owned layers whose bytes changed (empty for a replay).
    pub changed_layers: Vec<String>,
    /// The foreign layers of the new revision and their digests; they equal the base's.
    pub foreign_layers: Vec<(String, ContentDigest)>,
    /// The revision that holds the result.
    pub revision: RevisionSummary,
    /// The declared footprint; `None` for a replay.
    pub footprint: Option<Footprint>,
}

fn command_error(error: CommandError) -> PlateError {
    PlateError::from(error)
}

fn io_error(code: ErrorCode) -> PlateError {
    let message = match code {
        ErrorCode::NotFound => "the source file could not be opened",
        ErrorCode::LimitExceeded => "the source file is larger than 65536 bytes",
        _ => "the source path is not a readable regular local file",
    };
    PlateError::new(code, message)
}

fn read_all_prims(
    reader: &Reader,
    opened: &Opened,
    control: &beamcraft_layer_engine::Control,
) -> Result<Vec<Prim>, PlateError> {
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
    Ok(prims)
}

fn model_placement(p: &LayerPlacement) -> Placement {
    Placement {
        translation_mm: p.translation_mm,
        orientation_xyzw: p.orientation_xyzw,
    }
}

fn layer_placement(p: &Placement) -> LayerPlacement {
    LayerPlacement {
        translation_mm: p.translation_mm,
        orientation_xyzw: p.orientation_xyzw,
    }
}

pub(crate) fn wrapper_of(view: &ReferenceView) -> Result<Wrapper, PlateError> {
    let record = ReferenceRecord {
        id: ElementId::parse(&view.id)
            .map_err(|_| integrity("a wrapper identity is not a full UUIDv7"))?,
        name: view.name.clone(),
        placement: model_placement(&view.placement),
        source: view.source.clone(),
        units: view.units,
    };
    Ok(Wrapper {
        status: record.units_status(),
        layer: view.source.layer_name(),
        foreign: view.foreign.clone(),
        record,
    })
}

/// Everything the head says that these commands need: every element identity and every wrapper.
struct Head {
    element_ids: BTreeSet<String>,
    wrappers: Vec<Wrapper>,
}

fn read_head(base: &Base, control: &beamcraft_layer_engine::Control) -> Result<Head, PlateError> {
    let mut reader = Reader::default();
    let opened = reader.open(open_base(base)?, control)?;
    let prims = read_all_prims(&reader, &opened, control)?;
    let views = read_references(&prims)?;
    Ok(Head {
        element_ids: prims.iter().filter_map(|p| p.element_id.clone()).collect(),
        wrappers: views.iter().map(wrapper_of).collect::<Result<_, _>>()?,
    })
}

fn find(head: &Head, id: ElementId) -> Option<&Wrapper> {
    head.wrappers.iter().find(|w| w.record.id == id)
}

fn not_found() -> PlateError {
    command_error(CommandError::new(
        ErrorCode::NotFound,
        "target_id",
        "the reference is not in the head revision",
    ))
}

fn layer_name_of(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned)
}

/// Reads the source file and pins it: the exact bytes, their digest and size, the format and what
/// the layer declares. Nothing is composed and the file is only read.
fn pin_source(
    path: &str,
    control: &beamcraft_layer_engine::Control,
) -> Result<(SourcePin, Vec<u8>), PlateError> {
    let label = layer_name_of(path).ok_or(PlateError {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message: "the source path has no file name",
    })?;
    let cap = usize::try_from(MAX_SOURCE_BYTES).map_err(|_| integrity("source cap"))?;
    let bytes = read_file(path, cap).map_err(io_error)?;
    let info = inspect_foreign_layer(&bytes, Budgets::FIRST_SLICE, control)?;
    let default_prim = info.default_prim.ok_or(PlateError {
        rule_id: None,
        code: ErrorCode::UnsupportedProperty,
        message: "the source declares no defaultPrim; a root prim is never guessed",
    })?;
    let declared_up_axis = match info.up_axis.as_deref() {
        None => None,
        Some(text) => Some(UpAxis::parse(text).ok_or(PlateError {
            rule_id: None,
            code: ErrorCode::UnsupportedProperty,
            message: "the source declares an up axis other than Y or Z",
        })?),
    };
    let pin = SourcePin {
        digest: digest_of(&bytes)?,
        bytes: bytes.len() as u64,
        format: info.format,
        default_prim,
        version: 1,
        label,
        declared_meters_per_unit: info.meters_per_unit,
        declared_up_axis,
    };
    Ok((pin, bytes))
}

/// Attaches the foreign file at `source_path` as a read-only reference and publishes the next
/// revision, or replays the operation.
///
/// Steps, each failing closed: classify the path text (no I/O); read and examine the file; open
/// and verify the head; decide replay from the host ledger; engine admission; author the wrapper
/// with U02, adding the source as a non-owned layer of the stage; check the layer engine's writes
/// against the declared footprint; commit through U06/U07 with the strict verifier, which
/// re-validates that no owned layer holds an opinion on foreign content.
pub fn attach_reference(
    store: &RevisionStore,
    request: &AttachRequest,
    hook: &mut dyn CommitHook,
) -> Result<Attached, PlateError> {
    // First of all, and before the store or the file system is touched.
    check_source_path(&request.source_path).map_err(command_error)?;
    let control = control();
    let (mut pin, bytes) = pin_source(&request.source_path, &control)?;
    let name = request.name.clone().unwrap_or_else(|| pin.label.clone());
    let (head, base) = load_base(store, &request.ids.project, &request.ids.workspace)?;
    let layer_name = pin.layer_name();
    let source_reused = base.external.contains(&layer_name);
    let known = read_head(&base, &control)?;
    // A source already attached keeps its version counter: the same bytes are the same snapshot.
    if let Some(same) = known
        .wrappers
        .iter()
        .find(|w| w.record.source.digest == pin.digest)
    {
        pin.version = same.record.source.version;
    }
    let command = AttachReference {
        new_id: request.new_id,
        name,
        placement: request.placement,
        source: pin.clone(),
    };
    let fingerprint = command.content_fingerprint().map_err(command_error)?;
    let op_host = host_uuid(&request.ids.op_id.to_string())?;
    let (prior, recorded) = prior_op_with(store, &op_host, || {
        // The head holds the result of this attach exactly when the wrapper of that id is there.
        match find(&known, command.new_id) {
            Some(w) => Ok(AttachReference {
                new_id: command.new_id,
                name: w.record.name.clone(),
                placement: w.record.placement,
                source: w.record.source.clone(),
            }
            .content_fingerprint()
            .map_err(command_error)?),
            None => digest_of(b"no accepted request with this element id"),
        }
    })?;
    match prior {
        PriorOp::Accepted { fingerprint: seen } if seen == fingerprint => {
            let wrapper = find(&known, command.new_id)
                .cloned()
                .ok_or_else(|| integrity("replayed wrapper is absent from the head revision"))?;
            return Ok(Attached {
                replayed: true,
                wrapper,
                source_reused,
                revision: replay_summary(&head, recorded)?,
                footprint: None,
            });
        }
        PriorOp::Accepted { .. } => {
            return Err(command_error(CommandError::new(
                ErrorCode::OpIdReuse,
                "op_id",
                "operation id was accepted with different content",
            )));
        }
        PriorOp::Unseen => {}
    }
    let outcome = attach(&command, &|id| known.element_ids.contains(&id.to_string()))
        .map_err(command_error)?;
    let mut open = open_base(&base)?;
    let Base {
        project,
        workspace,
        generation: base_generation,
        expected,
        ..
    } = base;
    if !source_reused {
        open.assets.push(Asset {
            name: layer_name.clone(),
            digest: pin.digest,
            bytes,
        });
        open.manifest = open.manifest_digest()?;
    }
    let schema = schema_identity()?;
    let (mut author, opened) = Author::open(
        open,
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
            op_id: request.ids.op_id.to_string(),
            actor: request.ids.actor.to_string(),
            schema,
            resources: Vec::new(),
            changes: vec![Change::AttachReference(Reference {
                id: command.new_id.to_string(),
                name: command.name.clone(),
                placement: layer_placement(&command.placement),
                source: pin.clone(),
            })],
        },
        &control,
    )?;
    check_footprint(&prepared, &outcome.footprint)?;
    let committed = publish_revision(
        store,
        &PublishRequest {
            footprint: &outcome.footprint,
            fingerprint,
            prepared: &prepared,
            schema,
            project: &project,
            workspace: &workspace,
            revision_id: &request.ids.revision_id,
            op: op_host,
            base_generation,
            expected: &expected,
        },
        hook,
    )?;
    if committed.outcome == CommitOutcome::Replayed {
        return Err(integrity("an unseen operation was reported as replayed"));
    }
    let wrapper = Wrapper {
        status: outcome.record.units_status(),
        layer: layer_name,
        foreign: Vec::new(),
        record: outcome.record,
    };
    // The foreign prims come from a fresh read of what was committed, not from the candidate.
    let wrapper = reread(store, wrapper.record.id)?.unwrap_or(wrapper);
    Ok(Attached {
        replayed: false,
        wrapper,
        source_reused,
        revision: summarize(&committed.info),
        footprint: Some(outcome.footprint),
    })
}

/// Opens the head again, fully verified, and returns one wrapper as committed.
fn reread(store: &RevisionStore, id: ElementId) -> Result<Option<Wrapper>, PlateError> {
    let inspection = inspect_references(store)?;
    Ok(inspection
        .references
        .into_iter()
        .find(|w| w.record.id == id))
}

/// How a wrapper edit turns the wrapper as it is now into a change and a declared footprint.
struct Step {
    /// The fingerprint of the request.
    fingerprint: ContentDigest,
    /// The fingerprint an accepted request would have if the head already holds its result.
    head_fingerprint: ContentDigest,
    /// The layer engine change.
    change: Change,
    /// The wrapper after the edit.
    after: ReferenceRecord,
    /// The declared footprint.
    footprint: Footprint,
}

fn edit(
    store: &RevisionStore,
    ids: &Ids,
    target: ElementId,
    plan: impl FnOnce(&ReferenceRecord) -> Result<Step, CommandError>,
    hook: &mut dyn CommitHook,
) -> Result<Edited, PlateError> {
    let control = control();
    let (head, base) = load_base(store, &ids.project, &ids.workspace)?;
    if head.is_none() {
        return Err(not_found());
    }
    let before: std::collections::BTreeMap<String, ContentDigest> = base
        .assets
        .iter()
        .map(|a| (a.name.clone(), a.digest))
        .collect();
    let known = read_head(&base, &control)?;
    let current = find(&known, target).ok_or_else(not_found)?;
    let step = plan(&current.record).map_err(command_error)?;
    let op_host = host_uuid(&ids.op_id.to_string())?;
    let (prior, recorded) = prior_op_with(store, &op_host, || Ok(step.head_fingerprint))?;
    match prior {
        PriorOp::Accepted { fingerprint } if fingerprint == step.fingerprint => {
            return Ok(Edited {
                replayed: true,
                wrapper: current.clone(),
                changed_layers: Vec::new(),
                foreign_layers: foreign_layers(&base),
                revision: replay_summary(&head, recorded)?,
                footprint: None,
            });
        }
        PriorOp::Accepted { .. } => {
            return Err(command_error(CommandError::new(
                ErrorCode::OpIdReuse,
                "op_id",
                "operation id was accepted with different content",
            )));
        }
        PriorOp::Unseen => {}
    }
    let schema = schema_identity()?;
    let (mut author, opened) = Author::open(
        open_base(&base)?,
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
            op_id: ids.op_id.to_string(),
            actor: ids.actor.to_string(),
            schema,
            resources: Vec::new(),
            changes: vec![step.change],
        },
        &control,
    )?;
    check_footprint(&prepared, &step.footprint)?;
    let changed_layers = prepared
        .layers
        .iter()
        .filter(|l| before.get(&l.name) != Some(&l.digest))
        .map(|l| l.name.clone())
        .collect();
    let foreign = prepared
        .retained
        .iter()
        .map(|l| (l.name.clone(), l.digest))
        .collect();
    let committed = publish_revision(
        store,
        &PublishRequest {
            prepared: &prepared,
            schema,
            project: &base.project,
            workspace: &base.workspace,
            revision_id: &ids.revision_id,
            footprint: &step.footprint,
            fingerprint: step.fingerprint,
            op: op_host,
            base_generation: base.generation,
            expected: &base.expected,
        },
        hook,
    )?;
    if committed.outcome == CommitOutcome::Replayed {
        return Err(integrity("an unseen operation was reported as replayed"));
    }
    let mut wrapper = current.clone();
    wrapper.record = step.after;
    wrapper.status = wrapper.record.units_status();
    Ok(Edited {
        replayed: false,
        wrapper: reread(store, target)?.unwrap_or(wrapper),
        changed_layers,
        foreign_layers: foreign,
        revision: summarize(&committed.info),
        footprint: Some(step.footprint),
    })
}

fn foreign_layers(base: &Base) -> Vec<(String, ContentDigest)> {
    base.assets
        .iter()
        .filter(|a| base.external.contains(&a.name))
        .map(|a| (a.name.clone(), a.digest))
        .collect()
}

/// Moves a wrapper: replaces its rigid placement and nothing else. The foreign layer, the source
/// pin and the units interpretation are untouched, which the result lists.
pub fn move_wrapper(
    store: &RevisionStore,
    ids: &Ids,
    command: &MoveReference,
    hook: &mut dyn CommitHook,
) -> Result<Edited, PlateError> {
    edit(
        store,
        ids,
        command.target_id,
        |current| {
            let outcome = move_reference(current, command)?;
            Ok(Step {
                fingerprint: outcome.fingerprint,
                head_fingerprint: MoveReference {
                    target_id: current.id,
                    placement: current.placement,
                }
                .content_fingerprint()?,
                change: Change::Placement {
                    id: command.target_id.to_string(),
                    placement: layer_placement(&command.placement),
                },
                after: outcome.record,
                footprint: outcome.footprint,
            })
        },
        hook,
    )
}

/// Records (or clears) the explicit units and up-axis interpretation of a wrapper. This is the
/// R01 correction: an operation on the wrapper, stored with the digest of the source it was made
/// for; the source layer is not edited.
pub fn correct_wrapper_units(
    store: &RevisionStore,
    ids: &Ids,
    command: &CorrectReferenceUnits,
    hook: &mut dyn CommitHook,
) -> Result<Edited, PlateError> {
    edit(
        store,
        ids,
        command.target_id,
        |current| {
            let outcome = correct_units(current, command)?;
            Ok(Step {
                fingerprint: outcome.fingerprint,
                head_fingerprint: CorrectReferenceUnits {
                    target_id: current.id,
                    interpretation: current.units,
                }
                .content_fingerprint()?,
                change: Change::ReferenceUnits {
                    id: command.target_id.to_string(),
                    interpretation: command.interpretation,
                },
                after: outcome.record,
                footprint: outcome.footprint,
            })
        },
        hook,
    )
}

/// The verified head and its wrappers.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceInspection {
    /// The verified revision.
    pub revision: RevisionSummary,
    /// Every wrapper, by identity.
    pub references: Vec<Wrapper>,
    /// The external (foreign) layers of the revision: name, digest, size. Each was re-verified
    /// against its digest and size when the revision was opened.
    pub foreign_layers: Vec<(String, ContentDigest, u64)>,
}

/// One wrapper's R02 measurement, with the identification the report carries.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasuredReference {
    /// Identity of the measured wrapper.
    pub id: ElementId,
    /// Composed path of the wrapper element.
    pub path: String,
    /// Label of the wrapper.
    pub name: String,
    /// The measurement; `mesh: None` inside when the wrapper has no supported surface.
    pub measurement: ReferenceMeasurement,
}

/// The result of `measure_references`: one report of the verified head.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasurementReport {
    /// The verified revision the numbers were read from.
    pub revision: RevisionSummary,
    /// One measurement per wrapper, in wrapper id order.
    pub measurements: Vec<MeasuredReference>,
}

/// Measures the composed foreign surface of every wrapper (or of `id` alone) as the R02 result
/// objects: kernel certification of the supported profile, units correction and the full
/// wrapper placement applied, enclosures for a certified shell, no value otherwise. Read-only:
/// nothing is written, no identity is consumed, no revision is created, and the same head
/// always answers with the same numbers.
pub fn measure_references(
    store: &RevisionStore,
    id: Option<ElementId>,
) -> Result<MeasurementReport, PlateError> {
    let (revision, _reader, _opened, prims) = open_head(store)?;
    let mut measurements = Vec::new();
    for (view, geometry) in crate::reference_view::reference_surfaces(&prims)? {
        let wrapper = wrapper_of(&view)?;
        if id.is_some_and(|wanted| wanted != wrapper.record.id) {
            continue;
        }
        let measurement =
            measure_reference(&wrapper.record, &geometry, &mut |_| false).map_err(command_error)?;
        measurements.push(MeasuredReference {
            id: wrapper.record.id,
            path: view.path.clone(),
            name: wrapper.record.name.clone(),
            measurement,
        });
    }
    if id.is_some_and(|wanted| !measurements.iter().any(|m| m.id == wanted)) {
        return Err(not_found());
    }
    Ok(MeasurementReport {
        revision: summarize(revision.info()),
        measurements,
    })
}

fn open_head(store: &RevisionStore) -> Result<(Revision, Reader, Opened, Vec<Prim>), PlateError> {
    let revision = store.open()?;
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
    let request = crate::plate::open_request(
        content.root_layer(),
        content.project().as_str(),
        content.workspace().as_str(),
        revision.info().manifest_digest(),
        content.workspace_generation(),
        assets,
        &external,
    )?;
    let control = control();
    // The structural validator also scans the specs of every owned layer, so a layer edited
    // outside BeamCraft to carry an opinion on foreign content is refused on read as well.
    validate_layers(request.clone(), schema_identity()?, &control)?;
    let mut reader = Reader::default();
    let opened = reader.open(request, &control)?;
    let prims = read_all_prims(&reader, &opened, &control)?;
    Ok((revision, reader, opened, prims))
}

/// Opens the head with verification and lists its wrappers: the pinned source, the declared and
/// explicit units, the placement and the foreign prims composed below each. The source layer of
/// every wrapper must be an external entry of the revision with exactly the pinned digest and
/// size, or the read fails with `INTEGRITY_ERROR`.
pub fn inspect_references(store: &RevisionStore) -> Result<ReferenceInspection, PlateError> {
    let (revision, _reader, _opened, prims) = open_head(store)?;
    let content = revision.manifest().content();
    let mut wrappers = Vec::new();
    for view in read_references(&prims)? {
        let wrapper = wrapper_of(&view)?;
        let entry = content
            .entries()
            .iter()
            .find(|e| e.name() == wrapper.layer)
            .ok_or_else(|| integrity("the source layer is not in the manifest"))?;
        if entry.role() != LayerRole::External
            || entry.digest() != view.source.digest
            || entry.size() != view.source.bytes
        {
            return Err(integrity("the source layer differs from its pin"));
        }
        wrappers.push(wrapper);
    }
    Ok(ReferenceInspection {
        revision: summarize(revision.info()),
        references: wrappers,
        foreign_layers: content
            .entries()
            .iter()
            .filter(|e| e.role() == LayerRole::External)
            .map(|e| (e.name().to_owned(), e.digest(), e.size()))
            .collect(),
    })
}

/// A write aimed at foreign content: always refused, never applied. Nothing is written, no
/// identity is consumed and no revision is created.
///
/// The engine decides (`beamcraft_engine::reference::write_foreign`) and the layer engine, which
/// derives ownership from composition provenance, must agree for every target it knows: the
/// source layer and every existing foreign prim must be read-only for it too, or the call fails
/// with `INTEGRITY_ERROR` instead of reporting a refusal it cannot back.
pub fn refuse_foreign_write(
    store: &RevisionStore,
    command: &WriteForeign,
) -> Result<Infallible, PlateError> {
    command.validate_arguments().map_err(command_error)?;
    let (_revision, reader, opened, prims) = open_head(store)?;
    let views = read_references(&prims)?;
    let view = views
        .iter()
        .find(|v| v.id == command.reference_id.to_string())
        .ok_or_else(not_found)?;
    let wrapper = wrapper_of(view)?;
    let error = match write_foreign(&wrapper.record, command) {
        Ok(never) => match never {},
        Err(error) => error,
    };
    if error.code == ErrorCode::ForeignReadOnly {
        let verdict = match &command.target {
            ForeignTarget::SourceLayer => {
                Some(reader.refuse_layer_write(&opened.context, &view.source.layer_name()))
            }
            ForeignTarget::Prim { path, .. } => path
                .strip_prefix(&format!("/{}", view.source.default_prim))
                .map(|rest| format!("{}{rest}", view.source_root))
                .filter(|composed| view.foreign.iter().any(|f| f.path == *composed))
                .map(|composed| reader.refuse_write(&opened.context, &composed)),
        };
        // A prim that does not exist yet has no provenance to consult; every existing target
        // must be read-only for the layer engine as well.
        if let Some(verdict) = verdict
            && !matches!(verdict, Err(e) if e.code == ErrorCode::ForeignReadOnly)
        {
            return Err(integrity(
                "the layer engine does not confirm that the target is foreign",
            ));
        }
    }
    Err(command_error(error))
}
