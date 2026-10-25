// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! C01 adapter: build a changeset from a personal workspace, and receive one into the published
//! store idempotently.
//!
//! * [`create_changeset`] packages the unsent draft revisions of a workspace: the base revision
//!   (the workspace's fork or last publication), every unsent operation with the manifest digest
//!   of its draft revision, the complete layer set of the draft head with each layer's base
//!   digest, the changed layers as required blobs, and the aggregate captured command footprint
//!   (a conservative state-difference footprint for legacy intervals). Canonical bytes are a blob
//!   in the workspace store, so the changeset is addressed by its digest.
//! * [`apply_changeset`] decodes and validates a changeset, checks the claimed actor against the
//!   trusted caller (`PERMISSION_DENIED`), binds the changeset id to the payload digest in the
//!   published store before anything is published (a retry with the same payload is a replay
//!   or resumes; another payload under the id is `OP_ID_REUSE`), checks the base against the
//!   published head (`BASE_REVISION_MISMATCH`), copies and verifies the blobs, and commits the
//!   layers through U06/U07 with the strict verifier, with the changeset id as the operation.
//!
//! Honesty limits: the CLI's `--actor-id` is its trusted context; C02's opt-in coordinator derives
//! that context from its token. A moved head is refused; this does not implement C03 rebase or
//! C04 conflict resolution. A legacy or unverified source interval never earns precise evidence.

use crate::plate::{PlateError, RevisionSummary, integrity, open_request, summarize};
use crate::workspace::{WorkspaceMeta, read_meta, write_meta};
use beamcraft_host::revision::{
    CommitHook, CommitOutcome, ExpectedBase, LayerEntry, LayerRole as HostRole, Revision,
    RevisionDraft, RevisionError, RevisionStore, Uuid7,
};
use beamcraft_layer_engine::authored::schema_identity;
use beamcraft_layer_engine::{Asset as LayerAsset, Cancellation, Control, Prim, Reader};
use beamcraft_model::changeset::{
    Access, Asset, Changeset, Dependency, DependencyGroup, DependencyTarget, LayerDelta, LayerRole,
    MAX_CHANGESET_BYTES, MAX_OPERATIONS, Receipt,
};
use beamcraft_model::command::CommandError;
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ActorId, ElementId, OpId, ProjectId, RevisionId, WorkspaceId};
use beamcraft_model::limits::MAX_FOOTPRINT_ENTRIES;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{Duration, Instant};

/// Identities of one [`create_changeset`] call.
#[derive(Debug, Clone)]
pub struct CreateChangeset {
    /// The changeset (publication) identity.
    pub changeset_id: OpId,
    /// Identity the published revision will take.
    pub revision_id: RevisionId,
    /// Claimed author; the workspace user when not given.
    pub actor: Option<ActorId>,
}

/// A stored changeset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredChangeset {
    /// The decoded changeset.
    pub changeset: Changeset,
    /// Digest of its canonical bytes (its blob in the workspace store).
    pub digest: ContentDigest,
}

/// Result of [`apply_changeset`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// True when the same payload was already published and nothing was written.
    pub replayed: bool,
    /// The published revision.
    pub revision: RevisionSummary,
    /// Digest of the received payload.
    pub digest: ContentDigest,
    /// Whether the workspace base now names the published revision. `false` after a publication
    /// whose workspace update failed: the revision is published; a retry of the same changeset
    /// replays and repairs the workspace.
    pub workspace_synced: bool,
}

fn control() -> Control {
    Control {
        cancellation: Cancellation::default(),
        deadline: Instant::now() + Duration::from_secs(60),
    }
}

fn host_id(id: &Uuid7) -> Result<beamcraft_model::id::Uuid7, PlateError> {
    beamcraft_model::id::Uuid7::parse(id.as_str())
        .map_err(|_| integrity("identity is not a UUIDv7"))
}

fn as_host(text: &str) -> Result<Uuid7, PlateError> {
    Ok(Uuid7::parse(text)?)
}

fn role(role: HostRole) -> LayerRole {
    match role {
        HostRole::Authored => LayerRole::Authored,
        HostRole::Derived => LayerRole::Derived,
        HostRole::External => LayerRole::External,
    }
}

fn host_role(role: LayerRole) -> HostRole {
    match role {
        LayerRole::Authored => HostRole::Authored,
        LayerRole::Derived => HostRole::Derived,
        LayerRole::External => HostRole::External,
    }
}

/// Every prim of a committed revision, read through the bounded layer engine.
fn prims_of(revision: &Revision) -> Result<Vec<Prim>, PlateError> {
    let content = revision.manifest().content();
    let assets = content
        .entries()
        .iter()
        .map(|e| {
            Ok(LayerAsset {
                name: e.name().into(),
                digest: e.digest(),
                bytes: revision.layer_bytes(e.name())?,
            })
        })
        .collect::<Result<Vec<_>, PlateError>>()?;
    // Foreign sources (R01) stay read-only: only the external layers are not owned.
    let external = content
        .entries()
        .iter()
        .filter(|e| e.role() == HostRole::External)
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
    let mut reader = Reader::default();
    let opened = reader.open(request, &control)?;
    let mut prims = Vec::new();
    let mut cursor = None;
    loop {
        let page = reader.read(&opened.context, cursor.as_ref(), &control)?;
        prims.extend(page.items);
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    Ok(prims)
}

/// What an element is, as far as the footprint is concerned: its own prim's properties and
/// relationships by name, and its descendant prims (the derived result).
#[derive(Default, PartialEq)]
struct ElementState<'a> {
    properties: BTreeMap<&'a str, &'a beamcraft_layer_engine::Property>,
    relationships: BTreeMap<&'a str, &'a [String]>,
    display_name: Option<&'a str>,
    descendants: BTreeMap<&'a str, Content<'a>>,
}

/// The content of a descendant prim, without its provenance: a prim whose layer moved but whose
/// values did not is unchanged.
#[derive(PartialEq)]
struct Content<'a> {
    type_name: Option<&'a str>,
    active: bool,
    kind: Option<&'a str>,
    properties: &'a [beamcraft_layer_engine::Property],
    relationships: &'a [beamcraft_layer_engine::Relationship],
}

fn elements(prims: &[Prim]) -> BTreeMap<ElementId, ElementState<'_>> {
    let mut by_path: BTreeMap<&str, ElementId> = BTreeMap::new();
    for p in prims {
        if let Some(id) = p
            .element_id
            .as_deref()
            .and_then(|t| ElementId::parse(t).ok())
        {
            by_path.insert(p.path.as_str(), id);
        }
    }
    let mut out: BTreeMap<ElementId, ElementState<'_>> = BTreeMap::new();
    for p in prims {
        if let Some(id) = by_path.get(p.path.as_str()) {
            let state = out.entry(*id).or_default();
            state.properties = p.properties.iter().map(|v| (v.name.as_str(), v)).collect();
            state.relationships = p
                .relationships
                .iter()
                .map(|r| (r.name.as_str(), r.targets.as_slice()))
                .collect();
            state.display_name = p.display_name.as_deref();
        } else if let Some(id) = by_path
            .iter()
            .find(|(path, _)| {
                p.path
                    .strip_prefix(**path)
                    .is_some_and(|r| r.starts_with('/'))
            })
            .map(|(_, id)| id)
        {
            out.entry(*id).or_default().descendants.insert(
                p.path.as_str(),
                Content {
                    type_name: p.type_name.as_deref(),
                    active: p.active,
                    kind: p.kind.as_deref(),
                    properties: &p.properties,
                    relationships: &p.relationships,
                },
            );
        }
    }
    out
}

fn property_group(name: &str) -> Option<DependencyGroup> {
    if name == "bim:name" {
        Some(DependencyGroup::Label)
    } else if name.starts_with("steel:") || name == "bim:axisPointsMm" {
        Some(DependencyGroup::ShapeInputs)
    } else if name.starts_with("xformOp") {
        Some(DependencyGroup::Placement)
    } else {
        None
    }
}

fn definition_targets(state: &ElementState<'_>) -> Vec<ContentDigest> {
    state
        .relationships
        .iter()
        .filter(|(name, _)| matches!(**name, "bim:profile" | "bim:material"))
        .flat_map(|(_, targets)| targets.iter())
        .filter_map(|t| crate::definition::definition_digest(t))
        .collect()
}

/// The conservative footprint of a changeset (F04 section 8 groups).
///
/// Writes come from the difference between the base and the head model: a created or removed
/// element writes every group; a changed field writes its group (an unknown field: every group); a
/// placement change also writes `derived` (the world-measurement invalidation); a changed result
/// writes `derived`; new definitions are written.
///
/// Reads cannot come from the difference: an operation may read state it does not change (a copy
/// reads its unchanged source, a move reads the shape), and per-command footprints are not
/// journaled. Until they are, the changeset reads every group of every element and every
/// definition of the base and the head, which is a superset of any operation's reads.
fn footprint(base: &[Prim], head: &[Prim]) -> Result<Vec<Dependency>, PlateError> {
    let mut set = BTreeSet::new();
    let mut add = |target, group, access| {
        set.insert(Dependency {
            target,
            group,
            access,
        });
    };
    let (before, after) = (elements(base), elements(head));
    let ids: BTreeSet<ElementId> = before.keys().chain(after.keys()).copied().collect();
    for id in ids {
        let target = DependencyTarget::Element(id);
        for group in DependencyGroup::ALL {
            add(target, group, Access::Read);
        }
        let (old, new) = (before.get(&id), after.get(&id));
        if old == new {
            continue;
        }
        let mut groups = BTreeSet::new();
        match (old, new) {
            (Some(old), Some(new)) => {
                let names: BTreeSet<&str> = old
                    .properties
                    .keys()
                    .chain(new.properties.keys())
                    .copied()
                    .collect();
                for name in names {
                    if old.properties.get(name) != new.properties.get(name) {
                        match property_group(name) {
                            Some(group) => {
                                groups.insert(group);
                            }
                            None => groups.extend(DependencyGroup::ALL),
                        }
                    }
                }
                let names: BTreeSet<&str> = old
                    .relationships
                    .keys()
                    .chain(new.relationships.keys())
                    .copied()
                    .collect();
                for name in names {
                    if old.relationships.get(name) != new.relationships.get(name) {
                        groups.insert(DependencyGroup::Relationship);
                        match name {
                            "bim:material" => {
                                groups.insert(DependencyGroup::MaterialRef);
                            }
                            "bim:profile" => {
                                groups.insert(DependencyGroup::ShapeInputs);
                            }
                            _ => {}
                        }
                    }
                }
                if old.display_name != new.display_name {
                    groups.insert(DependencyGroup::Label);
                }
                if old.descendants != new.descendants {
                    groups.insert(DependencyGroup::Derived);
                }
            }
            _ => groups.extend(DependencyGroup::ALL),
        }
        if groups.contains(&DependencyGroup::Placement) {
            groups.insert(DependencyGroup::Derived);
        }
        for group in groups {
            add(target, group, Access::Write);
        }
        for digest in old.into_iter().chain(new).flat_map(definition_targets) {
            add(
                DependencyTarget::Definition(digest),
                DependencyGroup::Relationship,
                Access::Read,
            );
        }
    }
    let definitions = |prims: &[Prim]| -> BTreeSet<ContentDigest> {
        prims
            .iter()
            .filter_map(|p| crate::definition::definition_digest(&p.path))
            .collect()
    };
    let known = definitions(base);
    let current = definitions(head);
    for digest in known.union(&current) {
        add(
            DependencyTarget::Definition(*digest),
            DependencyGroup::Relationship,
            Access::Read,
        );
    }
    for digest in current.difference(&known) {
        add(
            DependencyTarget::Definition(*digest),
            DependencyGroup::Relationship,
            Access::Write,
        );
    }
    if set.len() > MAX_FOOTPRINT_ENTRIES {
        return Err(PlateError {
            code: ErrorCode::LimitExceeded,
            message: "the conservative changeset footprint exceeds 4096 entries",
        });
    }
    Ok(set.into_iter().collect())
}

/// Builds the changeset of a workspace's unsent revisions and stores its canonical bytes as a
/// blob in the workspace store.
///
/// `INVALID_ARGUMENT` when nothing is unsent. Repeating the call with the same identities on an
/// unchanged draft produces the same bytes and digest.
pub fn create_changeset(
    published: &RevisionStore,
    workspace: &RevisionStore,
    dir: &Path,
    request: &CreateChangeset,
) -> Result<StoredChangeset, PlateError> {
    let state = crate::workspace::WorkspaceState::open(dir)?;
    let meta = state.meta().ok_or_else(crate::workspace::unusable)?;
    let draft = workspace.open()?;
    let head_sequence = draft.info().sequence();
    if meta.synced_sequence == 0 || meta.rebase_sequence > head_sequence {
        return Err(integrity(
            "workspace metadata disagrees with its draft head",
        ));
    }
    if head_sequence <= meta.synced_sequence {
        return Err(PlateError {
            code: ErrorCode::InvalidArgument,
            message: "the workspace has nothing to send",
        });
    }
    let unsent = head_sequence - meta.synced_sequence;
    if usize::try_from(unsent).map_or(true, |n| n > MAX_OPERATIONS) {
        return Err(PlateError {
            code: ErrorCode::LimitExceeded,
            message: "too many unsent revisions for one changeset",
        });
    }
    let base = crate::workspace::published_base(published, meta)?;
    let content = draft.manifest().content();
    let base_content = base.manifest().content();
    if content.project() != base_content.project()
        || content.schema() != base_content.schema()
        || content.workspace() != &meta.workspace_id
    {
        return Err(integrity(
            "the draft context differs from its accepted base",
        ));
    }
    let operations =
        crate::command_evidence::interval(workspace, meta.synced_sequence, head_sequence)?;
    let evidence =
        crate::command_evidence::aggregate(workspace, meta.synced_sequence, &operations)?;
    let dependencies = match evidence.footprint.as_ref() {
        Some(footprint) => crate::command_evidence::dependencies(footprint),
        None => footprint(&prims_of(&base)?, &prims_of(&draft)?)?,
    };
    let base_layers: BTreeMap<&str, ContentDigest> = base
        .manifest()
        .content()
        .entries()
        .iter()
        .map(|e| (e.name(), e.digest()))
        .collect();
    let mut layers = Vec::new();
    let mut assets = BTreeSet::new();
    for entry in content.entries() {
        let delta = LayerDelta {
            name: entry.name().to_owned(),
            role: role(entry.role()),
            base: base_layers.get(entry.name()).copied(),
            digest: entry.digest(),
            size: entry.size(),
        };
        if delta.changed() {
            assets.insert(Asset {
                digest: delta.digest,
                size: delta.size,
            });
        }
        layers.push(delta);
    }
    layers.sort();
    let changeset = Changeset {
        changeset_id: request.changeset_id,
        project: ProjectId::from_uuid(host_id(base_content.project())?),
        workspace: WorkspaceId::from_uuid(host_id(&meta.workspace_id)?),
        actor: match request.actor {
            Some(actor) => actor,
            None => ActorId::from_uuid(host_id(&meta.user_id)?),
        },
        base_revision: RevisionId::from_uuid(host_id(&meta.base_revision)?),
        base_manifest: meta.base_manifest,
        revision: request.revision_id,
        operations,
        dependencies,
        layers,
        root_layer: content.root_layer().to_owned(),
        assets: assets.into_iter().collect(),
    };
    let bytes = changeset.encode()?;
    let (digest, _) = workspace.blobs().publish(&bytes).map_err(|e| PlateError {
        code: e.code(),
        message: "changeset blob could not be stored",
    })?;
    Ok(StoredChangeset { changeset, digest })
}

/// Reads, verifies and decodes a stored changeset.
pub fn read_changeset(
    workspace: &RevisionStore,
    digest: &ContentDigest,
) -> Result<StoredChangeset, PlateError> {
    let bytes = workspace
        .blobs()
        .read_capped(digest, MAX_CHANGESET_BYTES)
        .map_err(|e| PlateError {
            code: e.code(),
            message: "changeset blob is not available",
        })?;
    Ok(StoredChangeset {
        changeset: Changeset::decode(&bytes)?,
        digest: *digest,
    })
}

/// Moves the workspace base to the published revision of an accepted changeset of this
/// workspace, so later changesets start from it. Unsent work after the changeset stays unsent.
fn advance_workspace(
    workspace: &RevisionStore,
    dir: &Path,
    meta: WorkspaceMeta,
    changeset: &Changeset,
    published: &beamcraft_host::revision::RevisionInfo,
) -> Result<(), PlateError> {
    if changeset.workspace.to_string() != meta.workspace_id.as_str()
        || changeset.base_revision.to_string() != meta.base_revision.as_str()
    {
        return Ok(());
    }
    let last = changeset
        .operations
        .last()
        .ok_or_else(|| integrity("changeset without operations"))?;
    let head = workspace
        .head()?
        .ok_or_else(|| integrity("the workspace has no revision"))?;
    for sequence in meta.synced_sequence.saturating_add(1)..=head.sequence() {
        let info = workspace.info_at(sequence)?;
        if info.manifest_digest() == last.result {
            return write_meta(
                dir,
                Some(&meta),
                &WorkspaceMeta {
                    base_revision: published.revision().clone(),
                    base_manifest: published.manifest_digest(),
                    synced_sequence: sequence,
                    ..meta.clone()
                },
            );
        }
    }
    Ok(())
}

/// Receives a changeset stored in a workspace into the published store.
///
/// Order (F04 sections 6 and 8): bounded decode and validation; trusted actor; changeset-id
/// replay; durable claim of the id; base against the published head; blob copy and
/// verification; commit with the strict verifier. Any error before the commit leaves the
/// published head unchanged.
pub fn apply_changeset(
    published: &RevisionStore,
    workspace: &RevisionStore,
    dir: &Path,
    digest: &ContentDigest,
    trusted_actor: &ActorId,
    hook: &mut dyn CommitHook,
) -> Result<Applied, PlateError> {
    let StoredChangeset { changeset, digest } = read_changeset(workspace, digest)?;
    let id = as_host(&changeset.changeset_id.to_string())?;
    let receipt = changeset.admit(trusted_actor, published.changeset_claim(&id)?)?;
    // The workspace metadata is checked before anything is published, so a later workspace update
    // can fail for I/O or a concurrent metadata transition. A published result stays successful.
    let meta = read_meta(dir)?;
    if workspace.head()?.is_none_or(|head| {
        meta.synced_sequence == 0
            || meta.synced_sequence > head.sequence()
            || meta.rebase_sequence > head.sequence()
    }) {
        return Err(integrity("workspace metadata disagrees with its revisions"));
    }
    // An operation id committed for something else is never this changeset's publication.
    if let Some(info) = published_as(published, &id)? {
        if !publishes(&info, &changeset) {
            return Err(reuse());
        }
        if receipt == Receipt::Replay {
            return Ok(replayed(workspace, dir, &changeset, &info, digest));
        }
    }
    let head = published.open().map_err(|e| match e {
        RevisionError::NoRevision => integrity("the published store has no revision"),
        other => other.into(),
    })?;
    let content = head.manifest().content();
    if changeset.project.to_string() != content.project().as_str() {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "project",
            "the changeset is for another project",
        )
        .into());
    }
    // Bind the id before anything can be published for it.
    published.claim_changeset(&id, &digest)?;
    if head.info().revision().as_str() != changeset.base_revision.to_string()
        || head.info().manifest_digest() != changeset.base_manifest
    {
        // A concurrent receipt of the same payload may have published it meanwhile.
        if let Some(info) = published_as(published, &id)?
            && publishes(&info, &changeset)
        {
            return Ok(replayed(workspace, dir, &changeset, &info, digest));
        }
        return Err(PlateError {
            code: ErrorCode::BaseRevisionMismatch,
            message: "the published head is not the changeset's base",
        });
    }
    let source =
        crate::command_evidence::aggregate(workspace, meta.synced_sequence, &changeset.operations)?;
    let source_content = source.head.manifest().content();
    if source.head.sequence() < meta.rebase_sequence
        || source_content.project() != content.project()
        || source_content.workspace() != &meta.workspace_id
        || changeset.workspace.to_string() != meta.workspace_id.as_str()
        || changeset.base_revision.to_string() != meta.base_revision.as_str()
        || changeset.base_manifest != meta.base_manifest
        || source_content.schema() != content.schema()
        || source_content.root_layer() != changeset.root_layer
        || source_content.entries().len() != changeset.layers.len()
        || source_content
            .entries()
            .iter()
            .zip(&changeset.layers)
            .any(|(a, b)| {
                a.name() != b.name
                    || a.role() != host_role(b.role)
                    || a.digest() != b.digest
                    || a.size() != b.size
            })
    {
        return Err(integrity(
            "changeset output differs from its verified source interval",
        ));
    }
    let expected_dependencies = match source.footprint.as_ref() {
        Some(footprint) => crate::command_evidence::dependencies(footprint),
        None => {
            let source_revision = workspace.open_at(source.head.sequence())?;
            let mut expected = footprint(&prims_of(&head)?, &prims_of(&source_revision)?)?;
            // Old C01 v1 payloads had no mark group. They retain conservative validation and
            // never acquire a precise destination proof merely by passing the legacy path.
            if !changeset
                .dependencies
                .iter()
                .any(|d| d.group == DependencyGroup::Mark)
            {
                expected.retain(|d| d.group != DependencyGroup::Mark);
            }
            expected
        }
    };
    if changeset.dependencies != expected_dependencies {
        return Err(integrity(
            "changeset dependencies differ from verified source operations",
        ));
    }
    let head_layers: BTreeMap<&str, ContentDigest> = content
        .entries()
        .iter()
        .map(|e| (e.name(), e.digest()))
        .collect();
    let mut entries = Vec::new();
    for layer in &changeset.layers {
        if layer.base != head_layers.get(layer.name.as_str()).copied() {
            return Err(integrity("a layer's base digest is not the head's"));
        }
        let bytes = if layer.changed() {
            workspace.blobs().read(&layer.digest)
        } else {
            published.blobs().read(&layer.digest)
        }
        .map_err(|e| PlateError {
            code: e.code(),
            message: "a changeset layer is not available",
        })?;
        let entry = LayerEntry::from_bytes(&layer.name, host_role(layer.role), &bytes)?;
        if entry.digest() != layer.digest || entry.size() != layer.size {
            return Err(integrity(
                "a changeset layer differs from its declared identity",
            ));
        }
        entries.push(entry);
    }
    for asset in &changeset.assets {
        let bytes = workspace
            .blobs()
            .read(&asset.digest)
            .map_err(|e| PlateError {
                code: e.code(),
                message: "a changeset asset is not available",
            })?;
        if u64::try_from(bytes.len()).ok() != Some(asset.size) {
            return Err(integrity("a changeset asset has another size"));
        }
        published.blobs().publish(&bytes).map_err(|e| PlateError {
            code: e.code(),
            message: "a changeset asset could not be stored",
        })?;
    }
    let generation = content
        .workspace_generation()
        .checked_add(1)
        .ok_or(PlateError {
            code: ErrorCode::LimitExceeded,
            message: "generation exhausted",
        })?;
    let draft = RevisionDraft {
        project: content.project().clone(),
        workspace: content.workspace().clone(),
        revision: as_host(&changeset.revision.to_string())?,
        op: id,
        workspace_generation: generation,
        schema: schema_identity()?.digest,
        root_layer: changeset.root_layer.clone(),
        entries,
    };
    let evidence = source
        .footprint
        .as_ref()
        .map(|footprint| crate::command_evidence::store(published, &draft.op, digest, footprint))
        .transpose()?;
    let committed = crate::publication::commit_verified_with_evidence(
        published,
        &draft,
        &ExpectedBase::Revision(head.info().revision().clone()),
        evidence,
        hook,
    )?;
    let workspace_synced = sync_workspace(workspace, dir, &changeset, &committed.info);
    Ok(Applied {
        replayed: committed.outcome == CommitOutcome::Replayed,
        revision: summarize(&committed.info),
        digest,
        workspace_synced,
    })
}

fn reuse() -> PlateError {
    CommandError::new(
        ErrorCode::OpIdReuse,
        "changeset_id",
        "the changeset id names another published operation",
    )
    .into()
}

/// The revision the ledger proves was committed under this operation id, if any.
fn published_as(
    published: &RevisionStore,
    id: &Uuid7,
) -> Result<Option<beamcraft_host::revision::RevisionInfo>, PlateError> {
    match published.query_outcome(id)? {
        beamcraft_host::ledger::Outcome::Committed { sequence, .. } => {
            Ok(Some(published.info_at(sequence)?))
        }
        _ => Ok(None),
    }
}

/// Whether a committed revision is exactly this changeset's publication: its revision id, base
/// and complete layer set (names, roles, digests, sizes, root layer).
fn publishes(info: &beamcraft_host::revision::RevisionInfo, changeset: &Changeset) -> bool {
    let manifest = info.manifest();
    let content = manifest.content();
    let base = manifest.base().map(|b| b.revision.as_str().to_owned());
    let mut layers: Vec<(String, LayerRole, ContentDigest, u64)> = content
        .entries()
        .iter()
        .map(|e| (e.name().to_owned(), role(e.role()), e.digest(), e.size()))
        .collect();
    layers.sort();
    let mut expected: Vec<(String, LayerRole, ContentDigest, u64)> = changeset
        .layers
        .iter()
        .map(|l| (l.name.clone(), l.role, l.digest, l.size))
        .collect();
    expected.sort();
    info.revision().as_str() == changeset.revision.to_string()
        && base.as_deref() == Some(changeset.base_revision.to_string().as_str())
        && content.root_layer() == changeset.root_layer
        && layers == expected
}

fn replayed(
    workspace: &RevisionStore,
    dir: &Path,
    changeset: &Changeset,
    info: &beamcraft_host::revision::RevisionInfo,
    digest: ContentDigest,
) -> Applied {
    Applied {
        replayed: true,
        revision: summarize(info),
        digest,
        workspace_synced: sync_workspace(workspace, dir, changeset, info),
    }
}

/// Advances the workspace after a publication; a failure is reported, never turned into a
/// rejection of work that is already published.
fn sync_workspace(
    workspace: &RevisionStore,
    dir: &Path,
    changeset: &Changeset,
    info: &beamcraft_host::revision::RevisionInfo,
) -> bool {
    read_meta(dir)
        .and_then(|meta| advance_workspace(workspace, dir, meta, changeset, info))
        .is_ok()
}
