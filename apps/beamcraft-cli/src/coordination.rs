// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! F12 coordination adapter (U20 + U21): the project descriptor with its section/zone
//! registries and the pinned connections between independent projects.
//!
//! The descriptor lives in the root layer's `customLayerData` and is changed only by the
//! explicit commands of this module (PC-OWN-002); ordinary modelling commits carry it and
//! every pin unchanged. A connection pins an exact source revision of another store: the
//! command verifies the source revision completely and copies its whole dependency closure
//! into this store as `external` manifest entries (`pins/<connection>/<source layer>`), so a
//! coordination revision reopens without the source store and retained revisions keep their
//! pins through compaction (PC-REV-001, PC-KEEP-001). Updating a pin is an explicit command
//! that names the connection, the expected old pin and the candidate new pin, and publishes
//! against the expected coordination head (PC-COORD-002); the host commit makes the retry
//! idempotent and a concurrent change a `BASE_REVISION_MISMATCH`. Committing here uses the
//! established no-evidence path of forks and workspace publication; replay is decided by the
//! host ledger on the operation id.

use crate::assembly::read_prims;
use crate::level::LevelContext;
use crate::plate::{
    Base, PlateError, ROOT_LAYER, RevisionSummary, base_of, control, digest_of, host_uuid,
    integrity, load_base, numbers, open_base, open_request, property, summarize,
};
use crate::publication::commit_verified_with_evidence;
use beamcraft_host::ledger::Outcome;
use beamcraft_host::revision::{
    CommitHook, CommitOutcome, LayerEntry, LayerRole, Revision, RevisionError, RevisionStore,
};
use beamcraft_layer_engine::authored::descriptor::{descriptor_of, write_root_descriptor};
use beamcraft_layer_engine::authored::{SchemaIdentity, schema_identity, validate_layers};
use beamcraft_layer_engine::{Asset, Diagnostic, Prim, Reader};
use beamcraft_model::coordination::{SectionRecord, ZoneRecord};
use beamcraft_model::descriptor::{
    ConnectionPlacement, ConnectionRecord, ProjectDescriptor, ScopeSelection,
};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ConnectionId, ElementId, OpId, ProjectId, RevisionId, WorkspaceId};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Prefix of the pinned source layer entries of one coordination revision.
pub const PIN_PREFIX: &str = "pins/";

/// The role of a first-revision owned layer: only the derived layer is derived.
fn role_of(name: &str) -> LayerRole {
    if name == "derived.usda" {
        LayerRole::Derived
    } else {
        LayerRole::Authored
    }
}

fn diagnostic_fail(error: Diagnostic) -> PlateError {
    PlateError {
        rule_id: error.rule_id,
        code: error.code,
        message: error.message,
    }
}

fn model_fail(error: beamcraft_model::command::CommandError) -> PlateError {
    PlateError {
        rule_id: error.rule_id,
        code: error.code,
        message: error.message,
    }
}

fn limit_fail(message: &'static str) -> PlateError {
    PlateError {
        rule_id: None,
        code: ErrorCode::LimitExceeded,
        message,
    }
}

/// The applied answer to the exact retry of an already accepted operation: the recorded
/// revision, its descriptor and no changes (PC-COORD-002). `None` when the operation is new.
fn replayed_applied(store: &RevisionStore, op: &OpId) -> Result<Option<Applied>, PlateError> {
    let op = host_uuid(&op.to_string())?;
    if let Outcome::Committed { sequence, .. } =
        store.query_outcome(&op).map_err(dependency_error)?
    {
        let revision = store.open_at(sequence).map_err(dependency_error)?;
        let descriptor = descriptor_of_base(&base_of(&revision)?)?;
        return Ok(Some(Applied {
            replayed: true,
            revision: summarize(revision.info()),
            changed_layers: Vec::new(),
            descriptor: descriptor
                .ok_or_else(|| integrity("the replayed revision has no descriptor"))?,
        }));
    }
    Ok(None)
}

/// A pinned source layer prepared for publication: its source name and verified bytes.
struct PinnedLayer {
    source_name: String,
    bytes: Vec<u8>,
}

/// The coordination manifest entry name of one pinned source layer.
fn entry_name(connection: &ConnectionRecord, source_layer: &str) -> String {
    format!(
        "{PIN_PREFIX}{}/{source_layer}",
        connection.id.uuid().hex32()
    )
}

fn prefix_of(connection: ConnectionId) -> String {
    format!("{PIN_PREFIX}{}", connection.uuid().hex32())
}

/// Maps a source-store failure onto the F12 closure codes: a missing pinned blob is
/// `DEPENDENCY_UNAVAILABLE`, damaged bytes are `INTEGRITY_ERROR` (PC-REV-001).
fn dependency_error(error: RevisionError) -> PlateError {
    match error {
        RevisionError::MissingBlob { .. } => PlateError {
            rule_id: Some("PC-REV-001"),
            code: ErrorCode::DependencyUnavailable,
            message: "a pinned source layer blob is missing",
        },
        RevisionError::CorruptBlob { .. } | RevisionError::SizeMismatch { .. } => PlateError {
            rule_id: Some("PC-REV-001"),
            code: ErrorCode::IntegrityError,
            message: "a pinned source layer does not match its manifest",
        },
        other => other.into(),
    }
}

/// A fresh descriptor of this project with the admitted schema identity; both are known
/// valid, so only the schema text could fail and that failure is reported, not hidden.
fn fresh_descriptor(
    context: &LevelContext,
    schema: &SchemaIdentity,
) -> Result<ProjectDescriptor, PlateError> {
    ProjectDescriptor::new(context.project, &schema.digest.to_string()).map_err(model_fail)
}

/// Reads the descriptor of a base (the verified head, or the empty store's first-revision
/// layers), with full structural validation.
fn descriptor_of_base(base: &Base) -> Result<Option<ProjectDescriptor>, PlateError> {
    let control = control();
    let request = open_base(base)?;
    let schema = schema_identity()?;
    validate_layers(request.clone(), schema, &control)?;
    let mut reader = Reader::default();
    let opened = reader.open(request, &control)?;
    descriptor_of(&opened.metadata).map_err(diagnostic_fail)
}

/// The fully resolved pin of one source revision: the record to author and the verified bytes
/// to publish as this connection's closure.
struct ResolvedPin {
    record: ConnectionRecord,
    layers: Vec<PinnedLayer>,
}

/// The scope a pin request names: free text (a code resolved against the pinned revision's
/// own registry), or an already-resolved selection preserved from the connection's record and
/// re-verified by identity.
enum RequestedScope<'a> {
    Text(&'a str),
    Preserved(ScopeSelection),
}

/// Reads the pinned revision's descriptor, or an empty registry for a source that carries
/// none yet (only the whole project resolves there).
fn pinned_registry(revision: &Revision) -> Result<ProjectDescriptor, PlateError> {
    let control = control();
    let base = base_of(revision)?;
    let request = open_base(&base)?;
    let schema = schema_identity()?;
    validate_layers(request.clone(), schema, &control)?;
    let mut reader = Reader::default();
    let opened = reader.open(request, &control)?;
    match descriptor_of(&opened.metadata).map_err(diagnostic_fail)? {
        Some(descriptor) => Ok(descriptor),
        None => ProjectDescriptor::new(
            ProjectId::parse(revision.manifest().content().project().as_str())
                .map_err(|_| integrity("the source manifest project is not a UUIDv7"))?,
            &schema.digest.to_string(),
        )
        .map_err(model_fail),
    }
}

/// Opens, completely verifies and binds one source revision of another store: the exact
/// revision must exist there, belong to the claimed source project, and its own descriptor
/// must resolve or re-verify the scope (PC-ID-001, PC-COORD-003).
fn resolve_pin(
    connection: ConnectionId,
    source_dir: &Path,
    source_project: ProjectId,
    source_revision: RevisionId,
    scope: RequestedScope<'_>,
    placement: Option<ConnectionPlacement>,
    label: &str,
) -> Result<ResolvedPin, PlateError> {
    // An exact revision id is the only admitted pin: there is no `latest` (PC-COORD-001).
    let blobs = beamcraft_host::storage::BlobStore::new(source_dir).map_err(|e| PlateError {
        rule_id: None,
        code: e.code(),
        message: "the source store root is not usable",
    })?;
    let source = RevisionStore::new(blobs);
    let sequence = source
        .sequence_of(&host_uuid(&source_revision.to_string())?)
        .map_err(dependency_error)?
        .ok_or(PlateError {
            rule_id: None,
            code: ErrorCode::NotFound,
            message: "the source store has no such revision",
        })?;
    let revision = source.open_at(sequence).map_err(dependency_error)?;
    let manifest = revision.manifest();
    let content = manifest.content();
    if content.project().as_str() != source_project.to_string() {
        return Err(PlateError {
            rule_id: Some("PC-ID-002"),
            code: ErrorCode::InvalidArgument,
            message: "the source store revision belongs to another project",
        });
    }
    let unresolved = |message: &'static str| PlateError {
        rule_id: Some("PC-ID-001"),
        code: ErrorCode::InvalidArgument,
        message,
    };
    let scope = match scope {
        RequestedScope::Text("whole") => ScopeSelection::WholeProject,
        RequestedScope::Text(text) => pinned_registry(&revision)?
            .scope_of(text)
            .map_err(model_fail)?,
        RequestedScope::Preserved(ScopeSelection::WholeProject) => ScopeSelection::WholeProject,
        RequestedScope::Preserved(ScopeSelection::Section(id)) => {
            let registry = pinned_registry(&revision)?;
            if !registry
                .sections
                .iter()
                .any(|section| section.id == id.to_string())
            {
                return Err(unresolved(
                    "the preserved scope names no section of the pinned revision",
                ));
            }
            ScopeSelection::Section(id)
        }
        RequestedScope::Preserved(ScopeSelection::Zone(id)) => {
            let registry = pinned_registry(&revision)?;
            if !registry.zones.iter().any(|zone| zone.id == id.to_string()) {
                return Err(unresolved(
                    "the preserved scope names no zone of the pinned revision",
                ));
            }
            ScopeSelection::Zone(id)
        }
    };
    let mut layers = Vec::new();
    let mut record_layers = Vec::new();
    for entry in manifest.content().entries() {
        let bytes = revision
            .layer_bytes(entry.name())
            .map_err(dependency_error)?;
        record_layers.push((entry.name().to_owned(), entry.digest().to_string()));
        layers.push(PinnedLayer {
            source_name: entry.name().to_owned(),
            bytes,
        });
    }
    let record = ConnectionRecord {
        id: connection,
        source_project,
        source_workspace: WorkspaceId::from_uuid(
            beamcraft_model::id::Uuid7::parse(content.workspace().as_str())
                .map_err(|_| integrity("the source manifest workspace is not a UUIDv7"))?,
        ),
        source_revision,
        manifest: revision.info().manifest_digest().to_string(),
        layers: record_layers,
        scope,
        placement,
        label: label.to_owned(),
    };
    Ok(ResolvedPin { record, layers })
}

/// One applied descriptor change.
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    /// True when the same operation was already committed and nothing was written.
    pub replayed: bool,
    /// The revision that holds the result.
    pub revision: RevisionSummary,
    /// Names of the layers whose bytes changed (empty for a replay).
    pub changed_layers: Vec<String>,
    /// The resulting descriptor.
    pub descriptor: ProjectDescriptor,
}

/// What a commit does with the pin entries of one connection: carry them, replace them with
/// newly resolved bytes, or drop them.
enum PinChange {
    Carry,
    Replace(Vec<PinnedLayer>),
    Drop,
}

/// The shared descriptor commit: rewrite the root layer, carry every other base entry and pin,
/// apply the pin change, and publish against the expected head. The host commit is idempotent
/// by operation id and refuses a moved head with `BASE_REVISION_MISMATCH`.
fn commit_descriptor(
    store: &RevisionStore,
    context: &LevelContext,
    update: impl FnOnce(Option<ProjectDescriptor>) -> Result<ProjectDescriptor, PlateError>,
    change: PinChange,
    changed_connection: Option<ConnectionId>,
    hook: &mut dyn CommitHook,
) -> Result<Applied, PlateError> {
    let schema = schema_identity()?;
    let (head, base) = load_base(store, &context.project, &context.workspace)?;
    let descriptor = update(descriptor_of_base(&base)?)?;
    // The root layer with the new descriptor; every other base layer is carried unchanged.
    let root_asset = base
        .assets
        .iter()
        .find(|a| a.name == ROOT_LAYER)
        .ok_or_else(|| integrity("the base has no root layer"))?;
    let root_bytes = write_root_descriptor(&root_asset.bytes, Some(&descriptor))?;
    store.blobs().publish(&root_bytes).map_err(|e| PlateError {
        rule_id: None,
        code: e.code(),
        message: "the root layer blob could not be stored",
    })?;
    let mut entries = vec![LayerEntry::from_bytes(
        ROOT_LAYER,
        LayerRole::Authored,
        &root_bytes,
    )?];
    // The roles of the head's own entries: pins and foreign references stay external.
    let roles: std::collections::BTreeMap<&str, LayerRole> = head
        .as_ref()
        .map(|head| {
            head.manifest()
                .content()
                .entries()
                .iter()
                .map(|e| (e.name(), e.role()))
                .collect()
        })
        .unwrap_or_default();
    for asset in &base.assets {
        if asset.name == ROOT_LAYER || asset.name.starts_with(PIN_PREFIX) {
            continue;
        }
        store
            .blobs()
            .publish(&asset.bytes)
            .map_err(|e| PlateError {
                rule_id: None,
                code: e.code(),
                message: "a carried layer blob could not be stored",
            })?;
        let role = roles
            .get(asset.name.as_str())
            .copied()
            .unwrap_or_else(|| role_of(&asset.name));
        entries.push(LayerEntry::new(
            &asset.name,
            role,
            asset.digest,
            u64::try_from(asset.bytes.len())
                .map_err(|_| limit_fail("a carried layer is too large"))?,
        )?);
    }
    // Pin entries of the head manifest, adjusted by this change. Carried pins reference blobs
    // that are already in the store, so they are listed without republishing bytes.
    let carried: Vec<LayerEntry> = head
        .as_ref()
        .map(|head| {
            head.manifest()
                .content()
                .entries()
                .iter()
                .filter(|e| e.role() == LayerRole::External && e.name().starts_with(PIN_PREFIX))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    for entry in carried {
        let affected =
            changed_connection.is_some_and(|id| entry.name().starts_with(&prefix_of(id)));
        let keep = !matches!(change, PinChange::Replace(_) | PinChange::Drop) || !affected;
        if keep {
            entries.push(entry);
        }
    }
    if let PinChange::Replace(layers) = &change {
        let record = descriptor
            .connections
            .iter()
            .find(|c| Some(c.id) == changed_connection)
            .ok_or_else(|| integrity("the changed connection is not in the descriptor"))?;
        for layer in layers {
            store
                .blobs()
                .publish(&layer.bytes)
                .map_err(|e| PlateError {
                    rule_id: None,
                    code: e.code(),
                    message: "a pinned source layer blob could not be stored",
                })?;
            entries.push(LayerEntry::from_bytes(
                &entry_name(record, &layer.source_name),
                LayerRole::External,
                &layer.bytes,
            )?);
        }
    }
    // The record of every connection must agree with the entries this revision commits
    // (PC-COORD-002: pin and bytes must agree).
    for record in &descriptor.connections {
        for (source_layer, digest) in &record.layers {
            let name = entry_name(record, source_layer);
            let entry = entries
                .iter()
                .find(|e| e.name() == name)
                .ok_or(PlateError {
                    rule_id: Some("PC-REV-001"),
                    code: ErrorCode::DependencyUnavailable,
                    message: "a connection record has no pinned entry",
                })?;
            if entry.digest().to_string() != *digest {
                return Err(integrity(
                    "a connection record disagrees with its pinned entry",
                ));
            }
        }
    }
    let generation = base
        .generation
        .checked_add(1)
        .ok_or_else(|| limit_fail("the workspace generation is exhausted"))?;
    let draft = beamcraft_host::revision::RevisionDraft {
        project: host_uuid(&base.project)?,
        workspace: host_uuid(&base.workspace)?,
        revision: host_uuid(&context.revision_id.to_string())?,
        op: host_uuid(&context.op_id.to_string())?,
        workspace_generation: generation,
        schema: schema.digest,
        root_layer: ROOT_LAYER.to_owned(),
        entries,
    };
    let committed = commit_verified_with_evidence(store, &draft, &base.expected, None, hook)?;
    let before = base
        .assets
        .iter()
        .map(|a| (a.name.as_str(), a.digest))
        .collect::<std::collections::BTreeMap<_, _>>();
    let changed_layers = draft
        .entries
        .iter()
        .filter(|e| before.get(e.name()) != Some(&e.digest()))
        .map(|e| e.name().to_owned())
        .collect();
    Ok(Applied {
        replayed: committed.outcome == CommitOutcome::Replayed,
        revision: summarize(&committed.info),
        changed_layers,
        descriptor,
    })
}

/// Sets or replaces the human project metadata (PC-OWN-002: the explicit shared command).
pub fn set_metadata(
    store: &RevisionStore,
    context: &LevelContext,
    code: Option<&str>,
    name: Option<&str>,
    organization: Option<&str>,
    hook: &mut dyn CommitHook,
) -> Result<Applied, PlateError> {
    let schema = schema_identity()?;
    commit_descriptor(
        store,
        context,
        |current| {
            let mut descriptor = match current {
                Some(descriptor) => descriptor,
                None => fresh_descriptor(context, &schema)?,
            };
            descriptor
                .set_metadata(code, name, organization)
                .map_err(model_fail)?;
            Ok(descriptor)
        },
        PinChange::Carry,
        None,
        hook,
    )
}

/// Inserts a section or renames the section of this identity (PC-ID-001).
pub fn define_section(
    store: &RevisionStore,
    context: &LevelContext,
    section: SectionRecord,
    hook: &mut dyn CommitHook,
) -> Result<Applied, PlateError> {
    let schema = schema_identity()?;
    commit_descriptor(
        store,
        context,
        |current| {
            let mut descriptor = match current {
                Some(descriptor) => descriptor,
                None => fresh_descriptor(context, &schema)?,
            };
            descriptor
                .put_section(section.clone())
                .map_err(model_fail)?;
            Ok(descriptor)
        },
        PinChange::Carry,
        None,
        hook,
    )
}

/// Inserts a zone or renames the zone of this identity (PC-ID-001).
pub fn define_zone(
    store: &RevisionStore,
    context: &LevelContext,
    zone: ZoneRecord,
    hook: &mut dyn CommitHook,
) -> Result<Applied, PlateError> {
    let schema = schema_identity()?;
    commit_descriptor(
        store,
        context,
        |current| {
            let mut descriptor = match current {
                Some(descriptor) => descriptor,
                None => fresh_descriptor(context, &schema)?,
            };
            descriptor.put_zone(zone.clone()).map_err(model_fail)?;
            Ok(descriptor)
        },
        PinChange::Carry,
        None,
        hook,
    )
}

/// Arguments of one connection creation or explicit update.
pub struct PinRequest {
    /// Source store directory.
    pub source_store: PathBuf,
    /// Claimed source project identity (PC-ID-002: a bare store lookup is not enough).
    pub source_project: ProjectId,
    /// Exact source revision to pin.
    pub source_revision: RevisionId,
    /// Scope selection text (`whole` or a source section/zone code); `None` keeps an existing
    /// connection's scope.
    pub scope: Option<String>,
    /// Explicit rigid placement; `None` keeps an existing connection's placement or records an
    /// unknown alignment.
    pub placement: Option<ConnectionPlacement>,
    /// Human label; `None` keeps an existing connection's label.
    pub label: Option<String>,
}

/// Connects one source project revision as a new read-only connection occurrence.
pub fn connect_project(
    store: &RevisionStore,
    context: &LevelContext,
    connection: ConnectionId,
    request: &PinRequest,
    hook: &mut dyn CommitHook,
) -> Result<Applied, PlateError> {
    if let Some(placement) = &request.placement {
        placement.validate().map_err(model_fail)?;
    }
    let label = request.label.as_deref().ok_or(PlateError {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message: "--label is required for a new connection",
    })?;
    if let Some(applied) = replayed_applied(store, &context.op_id)? {
        return Ok(applied);
    }
    let resolved = resolve_pin(
        connection,
        &request.source_store,
        request.source_project,
        request.source_revision,
        RequestedScope::Text(request.scope.as_deref().unwrap_or("whole")),
        request.placement,
        label,
    )?;
    let schema = schema_identity()?;
    let record = resolved.record;
    commit_descriptor(
        store,
        context,
        |current| {
            let mut descriptor = match current {
                Some(descriptor) => descriptor,
                None => fresh_descriptor(context, &schema)?,
            };
            descriptor
                .put_connection(record.clone())
                .map_err(model_fail)?;
            Ok(descriptor)
        },
        PinChange::Replace(resolved.layers),
        Some(connection),
        hook,
    )
}

/// Explicitly updates one connection's pin: names the connection, the expected old pin and the
/// candidate; the candidate is resolved and verified completely and the result is published
/// against the expected coordination head (PC-COORD-002).
pub fn update_connection(
    store: &RevisionStore,
    context: &LevelContext,
    connection: ConnectionId,
    expected_revision: RevisionId,
    request: &PinRequest,
    hook: &mut dyn CommitHook,
) -> Result<Applied, PlateError> {
    if let Some(placement) = &request.placement {
        placement.validate().map_err(model_fail)?;
    }
    // The exact retry of an accepted operation replays its recorded outcome before any pin
    // check or candidate resolution (PC-COORD-002).
    if let Some(applied) = replayed_applied(store, &context.op_id)? {
        return Ok(applied);
    }
    let (_, base) = load_base(store, &context.project, &context.workspace)?;
    let current = descriptor_of_base(&base)?;
    let record = current
        .as_ref()
        .and_then(|d| d.connections.iter().find(|c| c.id == connection))
        .ok_or(PlateError {
            rule_id: Some("PC-COORD-002"),
            code: ErrorCode::NotFound,
            message: "no connection of this identity exists",
        })?
        .clone();
    if record.source_revision != expected_revision {
        return Err(PlateError {
            rule_id: Some("PC-COORD-002"),
            code: ErrorCode::DependencyConflict,
            message: "the expected source pin differs from the pinned revision",
        });
    }
    // Without --scope the saved selection is preserved as an identity and re-verified against
    // the candidate's own registry; it never widens to the whole project (PC-COORD-003).
    let scope = match request.scope.as_deref() {
        Some(text) => RequestedScope::Text(text),
        None => RequestedScope::Preserved(record.scope.clone()),
    };
    let resolved = resolve_pin(
        connection,
        &request.source_store,
        request.source_project,
        request.source_revision,
        scope,
        request.placement.or(record.placement),
        request.label.as_deref().unwrap_or(&record.label),
    )?;
    let schema = schema_identity()?;
    let record = resolved.record;
    commit_descriptor(
        store,
        context,
        |current| {
            let mut descriptor = match current {
                Some(descriptor) => descriptor,
                None => fresh_descriptor(context, &schema)?,
            };
            descriptor
                .put_connection(record.clone())
                .map_err(model_fail)?;
            Ok(descriptor)
        },
        PinChange::Replace(resolved.layers),
        Some(connection),
        hook,
    )
}

/// Removes one connection from the descriptor. The pinned bytes of older retained revisions
/// stay reachable (PC-KEEP-001); only this revision's manifest drops the entries.
pub fn remove_connection(
    store: &RevisionStore,
    context: &LevelContext,
    connection: ConnectionId,
    hook: &mut dyn CommitHook,
) -> Result<Applied, PlateError> {
    commit_descriptor(
        store,
        context,
        |current| {
            let mut descriptor = current.ok_or(PlateError {
                rule_id: Some("PC-KEEP-001"),
                code: ErrorCode::NotFound,
                message: "the store has no descriptor with connections",
            })?;
            descriptor
                .remove_connection(connection)
                .map_err(model_fail)?;
            Ok(descriptor)
        },
        PinChange::Drop,
        Some(connection),
        hook,
    )
}

/// One connection as reported by `show`, with its resolved pin entries.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectionStatus {
    /// The authored record.
    pub record: ConnectionRecord,
    /// Resolved pin entries as `(source layer, digest)`; a missing or disagreeing pin is a
    /// typed refusal, never a silently partial list.
    pub resolution: Result<Vec<(String, String)>, PlateError>,
}

/// The descriptor of the verified head with every connection resolved against its manifest.
pub fn show(
    store: &RevisionStore,
    context: &LevelContext,
) -> Result<(Option<ProjectDescriptor>, Vec<ConnectionStatus>), PlateError> {
    let (head, base) = load_base(store, &context.project, &context.workspace)?;
    let descriptor = descriptor_of_base(&base)?;
    let Some(descriptor) = descriptor else {
        return Ok((None, Vec::new()));
    };
    let mut statuses = Vec::new();
    for record in &descriptor.connections {
        let resolution = match &head {
            Some(head) => resolve_entries(head, record),
            None => Err(PlateError {
                rule_id: Some("PC-REV-001"),
                code: ErrorCode::DependencyUnavailable,
                message: "the store has no revision to resolve pins against",
            }),
        };
        statuses.push(ConnectionStatus {
            record: record.clone(),
            resolution,
        });
    }
    Ok((Some(descriptor), statuses))
}

fn resolve_entries(
    head: &Revision,
    record: &ConnectionRecord,
) -> Result<Vec<(String, String)>, PlateError> {
    let mut out = Vec::new();
    for (source_layer, digest) in &record.layers {
        let name = entry_name(record, source_layer);
        let entry = head
            .manifest()
            .content()
            .entries()
            .iter()
            .find(|e| e.name() == name)
            .ok_or(PlateError {
                rule_id: Some("PC-REV-001"),
                code: ErrorCode::DependencyUnavailable,
                message: "a pinned source layer entry is missing",
            })?;
        if entry.digest().to_string() != *digest {
            return Err(integrity(
                "a pinned entry disagrees with the connection record",
            ));
        }
        out.push((source_layer.clone(), entry.digest().to_string()));
    }
    Ok(out)
}

/// The descriptor of a revision's own root layer, with full structural validation.
pub(crate) fn revision_descriptor(
    revision: &Revision,
) -> Result<Option<ProjectDescriptor>, PlateError> {
    descriptor_of_base(&base_of(revision)?)
}

/// Opens one connection's pinned source revision as a standalone verified view of its own
/// identity (source project, workspace and manifest digest) and returns its prims. The pin
/// entries must resolve against `revision` and agree with the record (PC-REV-001). Pinned
/// bytes are foreign: they never join this store's own composition, so reads through this
/// view change nothing and no write can reach them as owned content.
pub(crate) fn pinned_prims(
    revision: &Revision,
    record: &ConnectionRecord,
) -> Result<Vec<Prim>, PlateError> {
    let control = control();
    resolve_entries(revision, record)?;
    let mut assets = Vec::new();
    for (source_layer, _) in &record.layers {
        let name = entry_name(record, source_layer);
        let bytes = revision.layer_bytes(&name).map_err(dependency_error)?;
        assets.push(Asset {
            digest: digest_of(&bytes)?,
            bytes,
            name,
        });
    }
    let request = open_request(
        &entry_name(record, ROOT_LAYER),
        &record.source_project.to_string(),
        &record.source_workspace.to_string(),
        record
            .manifest
            .parse::<ContentDigest>()
            .map_err(|_| integrity("the pinned manifest digest is malformed"))?,
        0,
        assets,
        &BTreeSet::new(),
    )?;
    let schema = schema_identity()?;
    validate_layers(request.clone(), schema, &control)?;
    let mut reader = Reader::default();
    let opened = reader.open(request, &control)?;
    read_prims(&reader, &opened.context, &control)
}

/// The verified prims of one connection's pinned source revision, with its record. The
/// connection must exist in the revision's descriptor.
pub(crate) fn pinned_prims_of(
    revision: &Revision,
    connection: ConnectionId,
) -> Result<(ConnectionRecord, Vec<Prim>), PlateError> {
    let descriptor = revision_descriptor(revision)?.ok_or(PlateError {
        rule_id: None,
        code: ErrorCode::NotFound,
        message: "the store has no project descriptor",
    })?;
    let record = descriptor
        .connections
        .iter()
        .find(|c| c.id == connection)
        .ok_or(PlateError {
            rule_id: None,
            code: ErrorCode::NotFound,
            message: "no connection of this identity exists",
        })?
        .clone();
    let prims = pinned_prims(revision, &record)?;
    Ok((record, prims))
}

/// Whether a pinned coordination connection of this store authors the element identity.
/// Pinned content is foreign bytes (PC-OWN-002): the identity exists there and every write
/// aimed at it is `FOREIGN_READ_ONLY`, never a local miss (T01.5).
pub(crate) fn pinned_owns_element(
    store: &RevisionStore,
    id: &ElementId,
) -> Result<bool, PlateError> {
    let head = match store.open() {
        Ok(revision) => revision,
        Err(RevisionError::NoRevision) => return Ok(false),
        Err(error) => return Err(dependency_error(error)),
    };
    let Some(descriptor) = revision_descriptor(&head)? else {
        return Ok(false);
    };
    let id = id.to_string();
    for record in &descriptor.connections {
        if pinned_prims(&head, record)?
            .iter()
            .any(|p| p.element_id.as_deref() == Some(id.as_str()))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// One pinned source element placed into the parent frame.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedElement {
    /// Source element identity.
    pub id: String,
    /// Position in the source frame, millimetres.
    pub source_mm: [f64; 3],
    /// Position in the parent frame, millimetres.
    pub parent_mm: [f64; 3],
}

/// The resolved coordination view of one connection: the pinned source elements' placements,
/// mapped by the connection's rigid transform into this project's frame (PC-FRAME-001). An
/// unknown alignment is an explicit unavailable state, never an identity placement. Nested
/// coordination chains compose from the source upward; this resolves one hop.
pub fn resolve(
    store: &RevisionStore,
    context: &LevelContext,
    connection: ConnectionId,
) -> Result<(ConnectionRecord, Vec<ResolvedElement>), PlateError> {
    let (_, base) = load_base(store, &context.project, &context.workspace)?;
    let descriptor = descriptor_of_base(&base)?.ok_or(PlateError {
        rule_id: None,
        code: ErrorCode::NotFound,
        message: "the store has no project descriptor",
    })?;
    let record = descriptor
        .connections
        .iter()
        .find(|c| c.id == connection)
        .ok_or(PlateError {
            rule_id: None,
            code: ErrorCode::NotFound,
            message: "no connection of this identity exists",
        })?
        .clone();
    let placement = record.placement.ok_or(PlateError {
        rule_id: Some("PC-FRAME-001"),
        code: ErrorCode::DependencyUnavailable,
        message: "the connection has no declared placement (unknown alignment)",
    })?;
    let head = store.open().map_err(dependency_error)?;
    let prims = pinned_prims(&head, &record)?;
    let mut elements = Vec::new();
    for prim in &prims {
        let Some(id) = prim.element_id.as_deref() else {
            continue;
        };
        let Some(source_mm) = property(prim, "xformOp:translate").and_then(numbers::<3>) else {
            continue;
        };
        let parent_mm = placement.apply_point(source_mm).map_err(model_fail)?;
        elements.push(ResolvedElement {
            id: id.to_owned(),
            source_mm,
            parent_mm,
        });
    }
    Ok((record, elements))
}
