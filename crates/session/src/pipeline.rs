// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The platform pipeline every command runs over: the layer names of a project, the base a new
//! revision is authored on (the verified head, or the empty initial model), the D1 project claim,
//! and the summary of a revision as commands report it.
//!
//! Nothing here knows about plates, members, bolts or any other element: a discipline authors its
//! elements over a [`Base`] and commits them, and this module is the part that is the same for
//! all of them.

use crate::Refusal;
use beamcraft_host::revision::{
    ExpectedBase, LayerRole, Manifest, Revision, RevisionError, RevisionInfo, RevisionStore, Uuid7,
};
use beamcraft_layer_engine::authored::PROJECT_RECORD_KEY;
use beamcraft_layer_engine::{
    Asset, BACKEND, Budgets, Cancellation, Control, Identity, OpenRequest, Profile,
};
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ProjectId, WorkspaceId};
use std::cell::Cell;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

/// Virtual name of the root layer.
pub const ROOT_LAYER: &str = "root.usda";
/// Virtual name of the authored (model) layer.
pub const AUTHORED_LAYER: &str = "authored.usda";
/// Virtual name of the derived (geometry) layer.
pub const DERIVED_LAYER: &str = "derived.usda";

/// The empty authored or derived layer of a first revision: a USD header with millimetre
/// units and Z up, no prims. The CLI's first-revision writer reads it from here.
pub const EMPTY_LAYER: &str = "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n)\n";

/// The root layer of the first revision of a project. It records the ProjectId (D1) in its
/// `customLayerData`, once: later revisions keep it, because the layer engine preserves the other
/// entries of that dictionary on every edit and no command writes it.
fn empty_root(project: &ProjectId) -> String {
    format!(
        "#usda 1.0\n(\n    customLayerData = {{\n        string {PROJECT_RECORD_KEY} = \"{project}\"\n    }}\n    defaultPrim = \"World\"\n    metersPerUnit = 0.001\n    subLayers = [@authored.usda@, @derived.usda@]\n    upAxis = \"Z\"\n)\n"
    )
}

/// An `INTEGRITY_ERROR` refusal with a fixed message.
pub fn integrity(message: &'static str) -> Refusal {
    Refusal::new(ErrorCode::IntegrityError, message)
}

/// The cancellation and the one-minute deadline a layer-engine call runs under.
pub fn control() -> Control {
    Control {
        cancellation: Cancellation::default(),
        deadline: Instant::now() + Duration::from_secs(60),
    }
}

/// The SHA-256 of `bytes`; `LIMIT_EXCEEDED` when they are too large to hash.
pub fn digest_of(bytes: &[u8]) -> Result<ContentDigest, Refusal> {
    Sha256::digest(bytes).map_err(|_| Refusal::new(ErrorCode::LimitExceeded, "layer too large"))
}

/// A host identity (UUIDv7) from its text form.
pub fn host_uuid(text: &str) -> Result<Uuid7, Refusal> {
    Uuid7::parse(text).map_err(Refusal::from)
}

/// The pin of the context that has no base revision.
pub fn initial_pin() -> Result<ContentDigest, Refusal> {
    digest_of(b"beamcraft-p01-initial-base")
}

/// The request that opens a stage over `assets` with `root` as its root layer. Every asset is
/// owned by the stage except the names in `external`, which stay read-only whatever their bytes say.
pub fn open_request(
    root: &str,
    project: &str,
    workspace: &str,
    revision: ContentDigest,
    generation: u64,
    assets: Vec<Asset>,
    external: &BTreeSet<String>,
) -> Result<OpenRequest, Refusal> {
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

fn asset(name: &str, bytes: &[u8]) -> Result<Asset, Refusal> {
    Ok(Asset {
        name: name.into(),
        digest: digest_of(bytes)?,
        bytes: bytes.to_vec(),
    })
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

/// The summary of a verified head revision, as the commands report it.
pub fn summarize(info: &RevisionInfo) -> RevisionSummary {
    summarize_manifest(info.manifest(), info.manifest_digest())
}

/// The summary of a manifest and its digest, for a revision that is not a stored head.
pub fn summarize_manifest(manifest: &Manifest, digest: ContentDigest) -> RevisionSummary {
    let content = manifest.content();
    RevisionSummary {
        revision_id: content.revision().as_str().to_owned(),
        sequence: manifest.sequence(),
        manifest_digest: digest,
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

/// The layers a new revision is authored over: the verified head, or the empty initial model.
pub struct Base {
    /// The project the layers belong to.
    pub project: String,
    /// The workspace that authors the next revision.
    pub workspace: String,
    /// The pin of the base: the manifest digest of the head, or a fixed one for the initial model.
    pub digest: ContentDigest,
    /// The workspace generation of the base.
    pub generation: u64,
    /// The bytes of every layer of the base.
    pub assets: Vec<Asset>,
    /// Names of the layers that are foreign sources: stored and composed, never owned or written.
    pub external: BTreeSet<String>,
    /// What the commit must find as the head: the base revision, or no revision at all.
    pub expected: ExpectedBase,
}

/// The layers of one stored revision as a base to open or author over.
pub fn base_of(revision: &Revision) -> Result<Base, Refusal> {
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
pub fn load_base(
    store: &RevisionStore,
    project: &ProjectId,
    workspace: &WorkspaceId,
) -> Result<(Option<Revision>, Base), Refusal> {
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

fn project_mismatch() -> Refusal {
    Refusal::new(
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
pub fn ensure_project(store: &RevisionStore, claimed: &ProjectId) -> Result<(), Refusal> {
    match store.open() {
        Ok(head) if head.manifest().content().project().as_str() == claimed.to_string() => Ok(()),
        Ok(_) => Err(project_mismatch()),
        Err(RevisionError::NoRevision) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// The pinned stage of a base, opened for reading.
pub fn open_base(base: &Base) -> Result<OpenRequest, Refusal> {
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
