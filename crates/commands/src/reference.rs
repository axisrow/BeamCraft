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
//! move / correct / update / inspect / write: the verified head, the wrapper, the same path
//! back; the update (R03) additionally carries the new snapshot as a manifest asset and
//! compares the supported properties and geometry of the two snapshots it switches between.
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
//! model-profile stage budgets (32 MiB per layer, 64 MiB total); its own sublayers, payloads,
//! variants and network references are refused. Replay of an `op_id` is decided from the head state,
//! as in the other commands. One implicit workspace, in-process layer engine, as in `plate`.

use crate::plate::{check_footprint, prior_op_with, replay_summary};
use crate::publication::{PublishRequest, changed_layers, publish_revision};
use crate::source_file::read_file;
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::measure::{ReferenceMeasurement, measure_reference};
use beamcraft_engine::reference::{
    ReferenceRecord, UnitsStatus, attach, correct_units, move_reference, update_source,
    write_foreign,
};
use beamcraft_host::revision::{
    CommitHook, CommitOutcome, LayerRole, Revision, RevisionStore, Uuid7,
};
use beamcraft_layer_engine::authored::{
    Change, ForeignPrim, Placement as LayerPlacement, Prepare, Reference, ReferenceView, SourcePin,
    UpAxis, read_references, schema_identity,
};
use beamcraft_layer_engine::{Asset, Budgets, Opened, Prim, Reader, inspect_foreign_layer};
use beamcraft_model::command::{CommandError, Placement};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId};
use beamcraft_model::reference::{
    AttachReference, CorrectReferenceUnits, ForeignTarget, MAX_SOURCE_BYTES, MoveReference,
    UpdateReferenceSource, WriteForeign, check_source_path,
};
use session::Refusal;
use session::pipeline::{
    Base, RevisionSummary, control, digest_of, host_uuid, integrity, load_base, open_base,
    summarize,
};
use std::collections::BTreeSet;
use std::convert::Infallible;
use steel::engine::transaction::PriorOp;

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

fn command_error(error: CommandError) -> Refusal {
    Refusal::from(error)
}

fn io_error(code: ErrorCode) -> Refusal {
    let message = match code {
        ErrorCode::NotFound => "the source file could not be opened",
        ErrorCode::LimitExceeded => "the source file is larger than 32 MiB",
        _ => "the source path is not a readable regular local file",
    };
    Refusal::new(code, message)
}

fn read_all_prims(
    reader: &Reader,
    opened: &Opened,
    control: &beamcraft_layer_engine::Control,
) -> Result<Vec<Prim>, Refusal> {
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

pub(crate) fn wrapper_of(view: &ReferenceView) -> Result<Wrapper, Refusal> {
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

fn read_head(base: &Base, control: &beamcraft_layer_engine::Control) -> Result<Head, Refusal> {
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

fn not_found() -> Refusal {
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
) -> Result<(SourcePin, Vec<u8>), Refusal> {
    let label = layer_name_of(path).ok_or(Refusal {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message: "the source path has no file name",
    })?;
    let cap = usize::try_from(MAX_SOURCE_BYTES).map_err(|_| integrity("source cap"))?;
    let bytes = read_file(path, cap).map_err(io_error)?;
    let info = inspect_foreign_layer(&bytes, Budgets::MODEL, control)?;
    let default_prim = info.default_prim.ok_or(Refusal {
        rule_id: None,
        code: ErrorCode::UnsupportedProperty,
        message: "the source declares no defaultPrim; a root prim is never guessed",
    })?;
    let declared_up_axis = match info.up_axis.as_deref() {
        None => None,
        Some(text) => Some(UpAxis::parse(text).ok_or(Refusal {
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
) -> Result<Attached, Refusal> {
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
    let (mut author, opened) = crate::scope::open_author(store, open, false, schema, &control)?;
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
fn reread(store: &RevisionStore, id: ElementId) -> Result<Option<Wrapper>, Refusal> {
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
) -> Result<Edited, Refusal> {
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
    let (mut author, opened) =
        crate::scope::open_author(store, open_base(&base)?, false, schema, &control)?;
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
    let changed_layers = changed_layers(&prepared, &before);
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
) -> Result<Edited, Refusal> {
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
) -> Result<Edited, Refusal> {
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

/// Inputs of one `update_wrapper_source` call (R03): the explicit re-read of a wrapper's source.
#[derive(Debug, Clone)]
pub struct UpdateRequest {
    /// Identities and context.
    pub ids: Ids,
    /// The wrapper whose source is replaced.
    pub target_id: ElementId,
    /// The path text of the new source file. Classified before any I/O and never stored.
    pub source_path: String,
}

/// One side of the R03 comparison: the supported properties of one pinned snapshot and its R02
/// measurement of the composed surface. What the supported profile cannot measure is stated in
/// `unsupported`, never guessed: the properties are still compared, the geometry is not.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceSide {
    /// Digest of the snapshot.
    pub digest: ContentDigest,
    /// Version of the snapshot.
    pub version: u32,
    /// Name of the snapshot's `defaultPrim`.
    pub default_prim: String,
    /// `metersPerUnit` the snapshot declares, if it declares one.
    pub declared_meters_per_unit: Option<f64>,
    /// `upAxis` the snapshot declares, if it declares one.
    pub declared_up_axis: Option<UpAxis>,
    /// Foreign prims composed below the wrapper.
    pub foreign_prim_count: usize,
    /// The kernel measurement of the composed surface (R02), when the supported profile
    /// certifies one.
    pub measurement: Option<ReferenceMeasurement>,
    /// The refusal code that stopped the measurement (`UNSUPPORTED_GEOMETRY`, a budget code,
    /// a status the profile reports without values), when there was one.
    pub unsupported: Option<ErrorCode>,
}

/// The R03 result's comparison of the two snapshots of one wrapper: the supported properties,
/// the certified geometry on both sides, and the composed prim tree the update added or removed.
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    /// The snapshot the wrapper pinned before the update.
    pub before: SourceSide,
    /// The snapshot the wrapper pins now.
    pub after: SourceSide,
    /// Source paths (`/DefaultPrim/...`) the new snapshot adds, bounded, in path order.
    pub added_prims: Vec<String>,
    /// Source paths the new snapshot removes, bounded, in path order.
    pub removed_prims: Vec<String>,
    /// True when a diff list was cut at the reply bound; the counts stay complete.
    pub prim_diff_truncated: bool,
    /// True when the update cleared an explicit units interpretation that was made for the
    /// earlier snapshot.
    pub units_cleared: bool,
}

/// Result of `update_wrapper_source`.
#[derive(Debug, Clone, PartialEq)]
pub struct Updated {
    /// True when the same operation was already committed and nothing was written.
    pub replayed: bool,
    /// The wrapper after the update.
    pub wrapper: Wrapper,
    /// The before/after comparison of the two snapshots.
    pub comparison: Comparison,
    /// Names of the owned layers whose bytes changed (empty for a replay).
    pub changed_layers: Vec<String>,
    /// The foreign layers of the new revision and their digests; both snapshots are there.
    pub foreign_layers: Vec<(String, ContentDigest)>,
    /// The revision that holds the result.
    pub revision: RevisionSummary,
    /// The declared footprint; `None` for a replay.
    pub footprint: Option<Footprint>,
}

/// The largest number of source paths one diff list carries; the counts stay complete, as with
/// the wrapper's own prim listing.
const MAX_LISTED_PRIM_DIFF: usize = 512;

/// Properties and prim paths of one side, before any measurement.
fn side_of(wrapper: &Wrapper) -> SourceSide {
    let source = &wrapper.record.source;
    SourceSide {
        digest: source.digest,
        version: source.version,
        default_prim: source.default_prim.clone(),
        declared_meters_per_unit: source.declared_meters_per_unit,
        declared_up_axis: source.declared_up_axis,
        foreign_prim_count: wrapper.foreign.len(),
        measurement: None,
        unsupported: None,
    }
}

/// Measures one wrapper (R02) over the verified head. A measurement the supported profile cannot
/// make is marked with its refusal code; a wrapper that is not there, or a store that fails to
/// open, is an error, not a mark.
fn measured_side(
    store: &RevisionStore,
    id: ElementId,
    wrapper: &Wrapper,
) -> Result<SourceSide, Refusal> {
    let mut side = side_of(wrapper);
    match measure_references(store, Some(id)) {
        Ok(report) => {
            let [measured] = report.measurements.as_slice() else {
                return Err(integrity("the measured wrapper is not uniquely reported"));
            };
            side.measurement = Some(measured.measurement.clone());
        }
        Err(refusal) if refusal.code == ErrorCode::NotFound => return Err(refusal),
        Err(refusal)
            if matches!(
                refusal.code,
                ErrorCode::IntegrityError | ErrorCode::BackendUnavailable
            ) =>
        {
            return Err(refusal);
        }
        Err(refusal) => side.unsupported = Some(refusal.code),
    }
    Ok(side)
}

fn prim_paths(wrapper: &Wrapper) -> std::collections::BTreeSet<&str> {
    wrapper
        .foreign
        .iter()
        .map(|f| f.source_path.as_str())
        .collect()
}

/// Explicitly re-reads the source of an existing wrapper: the file at `source_path` becomes the
/// wrapper's next pinned version, and nothing else about the wrapper moves. There is no
/// automatic update: a changed file on disk is never noticed until this command names it.
///
/// Steps, each failing closed: classify the path text (no I/O); read and examine the file; open
/// and verify the head; version policy (the wrapper's counter always moves one forward); replay
/// decision; engine admission; measure the before side; author the replacement with U02, adding
/// the new bytes as a non-owned manifest layer; check the layer engine's writes against the
/// declared footprint; publish and re-read; measure the after side and compare the supported
/// properties, geometry and prim trees of the two snapshots. The source files are only ever
/// read; an explicit units interpretation is cleared with the result stating it, because it is
/// stored with the digest it was made for.
pub fn update_wrapper_source(
    store: &RevisionStore,
    request: &UpdateRequest,
    hook: &mut dyn CommitHook,
) -> Result<Updated, Refusal> {
    // First of all, and before the store or the file system is touched.
    check_source_path(&request.source_path).map_err(command_error)?;
    let control = control();
    let (mut pin, bytes) = pin_source(&request.source_path, &control)?;
    let (head, base) = load_base(store, &request.ids.project, &request.ids.workspace)?;
    if head.is_none() {
        return Err(not_found());
    }
    let known = read_head(&base, &control)?;
    let current = find(&known, request.target_id).ok_or_else(not_found)?;
    // The pinned bytes are handled first: they are either a replay of the accepted update or,
    // for an unseen operation, the refusal "that snapshot is the current one". The version is
    // the wrapper's own counter of accepted snapshots: it moves one forward, and the digest
    // identifies the snapshot, never the count.
    let same_bytes = pin.digest == current.record.source.digest;
    pin.version = if same_bytes {
        current.record.source.version
    } else {
        current
            .record
            .source
            .version
            .checked_add(1)
            .ok_or(command_error(CommandError::new(
                ErrorCode::LimitExceeded,
                "source.version",
                "the version counter is exhausted",
            )))?
    };
    let command = UpdateReferenceSource {
        target_id: request.target_id,
        source: pin.clone(),
    };
    let fingerprint = command.content_fingerprint().map_err(command_error)?;
    let op_host = host_uuid(&request.ids.op_id.to_string())?;
    let (prior, recorded) = prior_op_with(store, &op_host, || {
        // The head holds the result of this update exactly when the wrapper already pins the
        // requested snapshot.
        UpdateReferenceSource {
            target_id: current.record.id,
            source: current.record.source.clone(),
        }
        .content_fingerprint()
        .map_err(command_error)
    })?;
    let layer_name = pin.layer_name();
    let source_reused = base.external.contains(&layer_name);
    match prior {
        PriorOp::Accepted { fingerprint: seen } if seen == fingerprint => {
            let side = measured_side(store, request.target_id, current)?;
            let wrapper = find(&known, request.target_id)
                .cloned()
                .ok_or_else(|| integrity("replayed wrapper is absent from the head revision"))?;
            return Ok(Updated {
                replayed: true,
                comparison: Comparison {
                    before: side.clone(),
                    after: side,
                    added_prims: Vec::new(),
                    removed_prims: Vec::new(),
                    prim_diff_truncated: false,
                    units_cleared: false,
                },
                wrapper,
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
    if same_bytes {
        return Err(command_error(CommandError::new(
            ErrorCode::InvalidArgument,
            "source.digest",
            "the source bytes are the pinned version",
        )));
    }
    let outcome = update_source(&current.record, &command).map_err(command_error)?;
    // Read-only, before anything is written: the measurement of the snapshot being replaced.
    let before = measured_side(store, request.target_id, current)?;
    let before_assets: std::collections::BTreeMap<String, ContentDigest> = base
        .assets
        .iter()
        .map(|a| (a.name.clone(), a.digest))
        .collect();
    let mut open = open_base(&base)?;
    if !source_reused {
        open.assets.push(Asset {
            name: layer_name.clone(),
            digest: pin.digest,
            bytes,
        });
        open.manifest = open.manifest_digest()?;
    }
    let schema = schema_identity()?;
    let (mut author, opened) = crate::scope::open_author(store, open, false, schema, &control)?;
    let prepared = author.prepare(
        Prepare {
            context: opened.context.clone(),
            op_id: request.ids.op_id.to_string(),
            actor: request.ids.actor.to_string(),
            schema,
            resources: Vec::new(),
            changes: vec![Change::ReferenceSource {
                id: request.target_id.to_string(),
                source: pin.clone(),
            }],
        },
        &control,
    )?;
    check_footprint(&prepared, &outcome.footprint)?;
    let changed = changed_layers(&prepared, &before_assets);
    let foreign = prepared
        .retained
        .iter()
        .map(|l| (l.name.clone(), l.digest))
        .collect();
    let committed = publish_revision(
        store,
        &PublishRequest {
            footprint: &outcome.footprint,
            fingerprint,
            prepared: &prepared,
            schema,
            project: &base.project,
            workspace: &base.workspace,
            revision_id: &request.ids.revision_id,
            op: op_host,
            base_generation: base.generation,
            expected: &base.expected,
        },
        hook,
    )?;
    if committed.outcome == CommitOutcome::Replayed {
        return Err(integrity("an unseen operation was reported as replayed"));
    }
    let wrapper = reread(store, request.target_id)?
        .ok_or_else(|| integrity("the updated wrapper is absent from the new head"))?;
    let after = measured_side(store, request.target_id, &wrapper)?;
    let (before_paths, after_paths) = (prim_paths(current), prim_paths(&wrapper));
    let added: Vec<_> = after_paths.difference(&before_paths).collect();
    let removed: Vec<_> = before_paths.difference(&after_paths).collect();
    let prim_diff_truncated =
        added.len() > MAX_LISTED_PRIM_DIFF || removed.len() > MAX_LISTED_PRIM_DIFF;
    Ok(Updated {
        replayed: false,
        comparison: Comparison {
            before,
            after,
            added_prims: added
                .into_iter()
                .take(MAX_LISTED_PRIM_DIFF)
                .map(|p| (*p).to_owned())
                .collect(),
            removed_prims: removed
                .into_iter()
                .take(MAX_LISTED_PRIM_DIFF)
                .map(|p| (*p).to_owned())
                .collect(),
            prim_diff_truncated,
            units_cleared: current.record.units.is_some(),
        },
        wrapper,
        changed_layers: changed,
        foreign_layers: foreign,
        revision: summarize(&committed.info),
        footprint: Some(outcome.footprint),
    })
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
) -> Result<MeasurementReport, Refusal> {
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

fn open_head(store: &RevisionStore) -> Result<(Revision, Reader, Opened, Vec<Prim>), Refusal> {
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
        .collect::<Result<Vec<_>, Refusal>>()?;
    let external = content
        .entries()
        .iter()
        .filter(|e| e.role() == LayerRole::External)
        .map(|e| e.name().to_owned())
        .collect();
    let request = session::pipeline::open_request(
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
    // `open_validated` runs those rules on the very stage it installs: one parse and compose
    // instead of two, with the checks, order, codes and messages of the two calls in sequence.
    let mut reader = Reader::default();
    let (opened, _) = reader.open_validated(request, schema_identity()?, &control)?;
    let prims = read_all_prims(&reader, &opened, &control)?;
    Ok((revision, reader, opened, prims))
}

/// Opens the head with verification and lists its wrappers: the pinned source, the declared and
/// explicit units, the placement and the foreign prims composed below each. The source layer of
/// every wrapper must be an external entry of the revision with exactly the pinned digest and
/// size, or the read fails with `INTEGRITY_ERROR`.
pub fn inspect_references(store: &RevisionStore) -> Result<ReferenceInspection, Refusal> {
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
) -> Result<Infallible, Refusal> {
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
