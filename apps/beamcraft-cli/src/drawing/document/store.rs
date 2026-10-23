// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Immutable drawing document revisions over the host's compare-and-swap revision journal.

use super::{Metadata, State};
use crate::drawing::{archive::SourceArchive, codec, derived};
use crate::plate::{PlateError, control, host_uuid, integrity};
use beamcraft_engine::drawing::Drawing;
use beamcraft_host::ledger::{Outcome, OutcomeSource};
use beamcraft_host::revision::{
    CommitOutcome, ExpectedBase, LayerEntry, LayerRole, Revision, RevisionDraft, RevisionInfo,
    RevisionStore,
};
use beamcraft_host::storage::BlobStore;
use beamcraft_layer_engine::{
    Asset, BACKEND, Budgets, Identity, OpenRequest, Profile, Reader, Value,
};
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ActorId, DocumentVersionId, DrawingId, OpId, Uuid7};
use std::fs;
use std::path::Path;

const INTENT_LAYER: &str = "drawing.usda";
const SOURCE_MANIFEST: &str = "source-manifest";
const INITIAL_REVISION: &[u8] = b"beamcraft-p01-initial-base";

/// A fully reopened document revision; every layer, source archive, and projection was verified.
#[derive(Debug)]
pub struct DocumentRevision {
    /// Host revision metadata.
    pub info: RevisionInfo,
    /// Drawing intent decoded from the authored USD.
    pub drawing: Drawing,
    /// Exact archived source model, independently inspected.
    pub source: SourceArchive,
    /// Draft or published lifecycle state.
    pub state: State,
    /// Immutable published version on this revision, if published.
    pub version: Option<DocumentVersionId>,
    /// Latest immutable published version retained by this document.
    pub latest_published: Option<DocumentVersionId>,
    /// Accepted request fingerprint.
    pub request: ContentDigest,
    /// Actor attribution.
    pub actor: ActorId,
    /// Host operation id.
    pub operation: OpId,
}

/// Document namespace and journal. Its root must be a dedicated directory, separate from models.
#[derive(Debug, Clone)]
pub struct DocumentStore {
    revisions: RevisionStore,
}

/// Caller-selected content and lifecycle transition for one document revision.
pub struct DocumentWrite<'a> {
    /// New immutable revision identity.
    pub revision: Uuid7,
    /// Idempotency operation.
    pub operation: OpId,
    /// Attribution identity.
    pub actor: ActorId,
    /// Admitted immutable drawing intent.
    pub drawing: &'a Drawing,
    /// Fully verified model resources for the intent's source pin.
    pub source: &'a SourceArchive,
    /// Draft or published state to commit.
    pub state: State,
    /// New immutable document version, only for publication.
    pub version: Option<DocumentVersionId>,
    /// Most recent published version retained by this revision.
    pub latest_published: Option<DocumentVersionId>,
}

fn bad(message: &'static str) -> PlateError {
    PlateError {
        code: ErrorCode::InvalidArgument,
        message,
    }
}

fn layer_role(name: &str) -> LayerRole {
    match name {
        "root.usda" | INTENT_LAYER => LayerRole::Authored,
        "drawing-derived.usda" => LayerRole::Derived,
        _ => LayerRole::External,
    }
}

fn field<'a>(
    prim: &'a beamcraft_layer_engine::Prim,
    name: &str,
    kind: &str,
) -> Result<&'a Value, PlateError> {
    let prop = prim
        .properties
        .iter()
        .find(|prop| prop.name == name && prop.type_name == kind)
        .ok_or_else(|| integrity("document metadata field is absent or mistyped"))?;
    prop.value
        .as_ref()
        .ok_or_else(|| integrity("document metadata field has no value"))
}

fn field_text<'a>(
    prim: &'a beamcraft_layer_engine::Prim,
    name: &str,
    kind: &str,
) -> Result<&'a str, PlateError> {
    match field(prim, name, kind)? {
        Value::Text(value) => Ok(value),
        _ => Err(integrity("document metadata field is not text")),
    }
}

fn metadata_from_root(root: &Asset, revision: &Revision) -> Result<Metadata, PlateError> {
    let content = revision.manifest().content();
    let identity = Identity {
        project: content.project().to_string(),
        workspace: content.workspace().to_string(),
        revision: revision
            .manifest()
            .base()
            .map_or_else(
                || Sha256::digest(INITIAL_REVISION),
                |base| Ok(base.manifest),
            )
            .map_err(|_| integrity("initial drawing revision digest failed"))?,
        generation: content.workspace_generation(),
    };
    let mut reader = Reader::default();
    let mut request = OpenRequest {
        identity,
        root: root.name.clone(),
        assets: revision
            .manifest()
            .content()
            .entries()
            .iter()
            .filter(|entry| {
                entry.name() == "root.usda"
                    || entry.name() == INTENT_LAYER
                    || entry.name() == "drawing-derived.usda"
            })
            .map(|entry| {
                Ok(Asset {
                    name: entry.name().to_owned(),
                    digest: entry.digest(),
                    bytes: revision.layer_bytes(entry.name())?,
                })
            })
            .collect::<Result<Vec<_>, PlateError>>()?,
        owned_layers: ["root.usda", INTENT_LAYER, "drawing-derived.usda"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        manifest: root.digest,
        backend: BACKEND,
        profile: Profile::StaticLocal,
        budgets: Budgets::FIRST_SLICE,
    };
    request.manifest = request.manifest_digest().map_err(PlateError::from)?;
    let control = control();
    let opened = reader.open(request, &control)?;
    let page = reader.read(&opened.context, None, &control)?;
    if page.next.is_some() {
        return Err(integrity("document root exceeds its prim budget"));
    }
    let prim = page
        .items
        .iter()
        .find(|prim| prim.path == "/Document" && prim.active)
        .ok_or_else(|| integrity("document metadata prim is missing"))?;
    if field_text(prim, "drawing:documentSchema", "string")?
        != super::schema_identity()?.to_string()
    {
        return Err(integrity("document schema identity disagrees"));
    }
    let state = match field_text(prim, "drawing:state", "token")? {
        "draft" => State::Draft,
        "published" => State::Published,
        _ => return Err(integrity("document state is unknown")),
    };
    let version_text = field_text(prim, "drawing:version", "string")?;
    let latest_text = field_text(prim, "drawing:latestPublished", "string")?;
    let parse_optional = |value: &str| {
        if value.is_empty() {
            Ok(None)
        } else {
            DocumentVersionId::parse(value)
                .map(Some)
                .map_err(|_| integrity("document version id is malformed"))
        }
    };
    let request = field_text(prim, "drawing:request", "string")?
        .parse::<ContentDigest>()
        .map_err(|_| integrity("document request digest is malformed"))?;
    let actor = ActorId::parse(field_text(prim, "drawing:actor", "string")?)
        .map_err(|_| integrity("document actor id is malformed"))?;
    let operation = OpId::parse(field_text(prim, "drawing:operation", "string")?)
        .map_err(|_| integrity("document operation id is malformed"))?;
    Ok(Metadata {
        state,
        version: parse_optional(version_text)?,
        latest_published: parse_optional(latest_text)?,
        request,
        actor,
        operation,
    })
}

impl DocumentStore {
    /// Creates a new dedicated document directory. Existing paths are never adopted implicitly.
    pub fn create(root: &Path) -> Result<Self, PlateError> {
        fs::create_dir(root).map_err(|_| bad("document directory could not be created"))?;
        Self::open(root)
    }

    /// Opens an existing dedicated document directory.
    pub fn open(root: &Path) -> Result<Self, PlateError> {
        let blobs = BlobStore::new(root).map_err(|_| bad("document directory is not usable"))?;
        Ok(Self {
            revisions: RevisionStore::new(blobs),
        })
    }

    /// Reopens and strictly verifies the current immutable document revision.
    pub fn head(&self) -> Result<DocumentRevision, PlateError> {
        let revision = self.revisions.open()?;
        self.verify_revision(revision)
    }

    /// Reopens every retained sequence in order; compacted history is omitted explicitly.
    pub fn history(&self) -> Result<Vec<DocumentRevision>, PlateError> {
        let Some(head) = self.revisions.head()? else {
            return Ok(Vec::new());
        };
        let mut revisions = Vec::new();
        for sequence in 1..=head.sequence() {
            match self.revisions.open_at(sequence) {
                Ok(revision) => revisions.push(self.verify_revision(revision)?),
                Err(beamcraft_host::revision::RevisionError::Compacted) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(revisions)
    }

    fn verify_revision(&self, revision: Revision) -> Result<DocumentRevision, PlateError> {
        let content = revision.manifest().content();
        if content.schema() != super::schema_identity()?
            || content.root_layer() != "root.usda"
            || content.entries().len() < 4
            || content.entries().len() > Budgets::MODEL.layers + 4
        {
            return Err(integrity(
                "document revision manifest is outside its schema",
            ));
        }
        let entries = content.entries();
        let mut assets = Vec::with_capacity(entries.len());
        let mut names = std::collections::BTreeSet::new();
        for entry in entries {
            if !names.insert(entry.name().to_owned()) || entry.role() != layer_role(entry.name()) {
                return Err(integrity("document revision layer names or roles disagree"));
            }
            let bytes = revision.layer_bytes(entry.name())?;
            assets.push(Asset {
                name: entry.name().to_owned(),
                digest: entry.digest(),
                bytes,
            });
        }
        let find = |name: &str| {
            assets
                .iter()
                .find(|asset| asset.name == name)
                .cloned()
                .ok_or_else(|| integrity("document layer is missing"))
        };
        let root = find("root.usda")?;
        let intent = find(INTENT_LAYER)?;
        let derived_asset = find("drawing-derived.usda")?;
        let metadata = metadata_from_root(&root, &revision)?;
        super::verify(&root, metadata)?;
        let digest = revision
            .manifest()
            .base()
            .map_or_else(
                || Sha256::digest(INITIAL_REVISION),
                |base| Ok(base.manifest),
            )
            .map_err(|_| integrity("initial drawing revision digest failed"))?;
        let identity = Identity {
            project: content.project().to_string(),
            workspace: content.workspace().to_string(),
            revision: digest,
            generation: content.workspace_generation(),
        };
        let drawing = codec::decode(intent, identity, &control())?;
        if drawing.spec().id.to_string() != content.workspace().to_string()
            || drawing.spec().source.project.to_string() != content.project().to_string()
        {
            return Err(integrity("document identity differs from its drawing"));
        }
        let resources = assets
            .into_iter()
            .filter(|asset| asset.name == SOURCE_MANIFEST || asset.name.starts_with("model/"))
            .collect();
        let source = SourceArchive::verify(drawing.spec().source, resources)?;
        if source.pin() != drawing.spec().source {
            return Err(integrity("document source archive pin differs"));
        }
        derived::verify(&derived_asset, &drawing, &source.inspect()?.snapshot)?;
        let roles: Vec<_> = entries
            .iter()
            .map(|entry| (entry.name(), entry.role()))
            .collect();
        if roles
            .iter()
            .filter(|(name, _)| *name == "root.usda")
            .count()
            != 1
            || roles
                .iter()
                .filter(|(name, _)| *name == INTENT_LAYER)
                .count()
                != 1
            || roles
                .iter()
                .filter(|(name, _)| *name == "drawing-derived.usda")
                .count()
                != 1
        {
            return Err(integrity("document revision layer set is incomplete"));
        }
        Ok(DocumentRevision {
            info: revision.info().clone(),
            drawing,
            source,
            state: metadata.state,
            version: metadata.version,
            latest_published: metadata.latest_published,
            request: metadata.request,
            actor: metadata.actor,
            operation: metadata.operation,
        })
    }

    /// Commits one already archived drawing snapshot with a host compare-and-swap.
    pub fn commit(
        &self,
        expected: &ExpectedBase,
        write: DocumentWrite<'_>,
    ) -> Result<(CommitOutcome, DocumentRevision), PlateError> {
        let DocumentWrite {
            revision,
            operation,
            actor,
            drawing,
            source,
            state,
            version,
            latest_published,
        } = write;
        if drawing.spec().source != source.pin() {
            return Err(integrity("drawing source pin differs from archived model"));
        }
        if (state == State::Published) != version.is_some()
            || (state == State::Published && latest_published != version)
        {
            return Err(bad("published document version metadata is inconsistent"));
        }
        let request =
            request_fingerprint(drawing, state, version, latest_published, actor, expected)?;
        let replay = match self.query(&operation) {
            Err(error) if error.code == ErrorCode::OutcomeUnknown => None,
            result => result?,
        };
        if let Some(previous) = replay {
            if previous.request != request {
                return Err(PlateError {
                    code: ErrorCode::OpIdReuse,
                    message: "document operation id was already used for another request",
                });
            }
            return Ok((CommitOutcome::Replayed, previous));
        }
        let previous = match expected {
            ExpectedBase::Initial => None,
            ExpectedBase::Revision(id) => {
                let sequence = self.revisions.sequence_of(id)?.ok_or(PlateError {
                    code: ErrorCode::BaseRevisionMismatch,
                    message: "expected document base is unavailable",
                })?;
                Some(self.verify_revision(self.revisions.open_at(sequence)?)?)
            }
        };
        let generation = previous
            .as_ref()
            .map_or(Some(1), |revision| {
                revision
                    .info
                    .manifest()
                    .content()
                    .workspace_generation()
                    .checked_add(1)
            })
            .ok_or_else(|| bad("document generation exhausted"))?;
        match (state, version, latest_published, previous.as_ref()) {
            (State::Draft, None, None, None) => {}
            (State::Draft, None, latest, Some(head)) if latest == head.latest_published => {}
            (State::Published, Some(version), Some(latest), Some(head))
                if version == latest
                    && head.state == State::Draft
                    && head.latest_published != Some(version) => {}
            _ => return Err(bad("document lifecycle transition is invalid")),
        }
        let metadata = Metadata {
            state,
            version,
            latest_published,
            request,
            actor,
            operation,
        };
        let root = super::encode(metadata)?;
        let intent = codec::encode(drawing)?;
        let projected = derived::encode(drawing, &source.inspect()?.snapshot)?;
        let mut assets = vec![root, intent, projected];
        assets.extend_from_slice(source.resources());
        let mut layer_entries = Vec::with_capacity(assets.len());
        for asset in &assets {
            self.revisions
                .blobs()
                .publish(&asset.bytes)
                .map_err(|error| PlateError {
                    code: error.code(),
                    message: "document layer could not be stored",
                })?;
            layer_entries.push(LayerEntry::from_bytes(
                &asset.name,
                layer_role(&asset.name),
                &asset.bytes,
            )?);
        }
        let draft = RevisionDraft {
            project: host_uuid(&drawing.spec().source.project.to_string())?,
            workspace: host_uuid(&drawing.spec().id.to_string())?,
            revision: host_uuid(&revision.to_string())?,
            op: host_uuid(&operation.to_string())?,
            workspace_generation: generation,
            schema: super::schema_identity()?,
            root_layer: "root.usda".to_owned(),
            entries: layer_entries,
        };
        let committed = self.revisions.commit(&draft, expected)?;
        let opened = self.revisions.open_at(committed.info.sequence())?;
        let verified = self.verify_revision(opened)?;
        Ok((committed.outcome, verified))
    }

    /// Answers a previously submitted operation from the host outcome ledger.
    pub fn query(&self, operation: &OpId) -> Result<Option<DocumentRevision>, PlateError> {
        match self
            .revisions
            .query_outcome(&host_uuid(&operation.to_string())?)?
        {
            Outcome::Committed {
                sequence,
                source: OutcomeSource::Ledger | OutcomeSource::Root,
                ..
            } => {
                let revision = self.revisions.open_at(sequence)?;
                let checked = self.verify_revision(revision)?;
                if checked.operation != *operation {
                    return Err(integrity(
                        "host outcome points to a different document operation",
                    ));
                }
                Ok(Some(checked))
            }
            Outcome::Aborted { .. } | Outcome::NotFound => Ok(None),
            Outcome::Unknown => Err(PlateError {
                code: ErrorCode::OutcomeUnknown,
                message: "document operation outcome is unknown",
            }),
        }
    }
}

/// Computes the operation fingerprint from the exact intent, lifecycle request, actor, and base.
pub fn request_fingerprint(
    drawing: &Drawing,
    state: State,
    version: Option<DocumentVersionId>,
    latest_published: Option<DocumentVersionId>,
    actor: ActorId,
    expected: &ExpectedBase,
) -> Result<ContentDigest, PlateError> {
    let intent = codec::encode(drawing)?;
    let mut bytes = Vec::with_capacity(intent.bytes.len() + 128);
    bytes.extend_from_slice(b"beamcraft-drawing-request/1\0");
    bytes.extend_from_slice(intent.digest.as_bytes());
    bytes.push(match state {
        State::Draft => 0,
        State::Published => 1,
    });
    match version {
        Some(id) => {
            bytes.push(1);
            bytes.extend_from_slice(id.to_string().as_bytes());
        }
        None => bytes.push(0),
    }
    match latest_published {
        Some(id) => {
            bytes.push(1);
            bytes.extend_from_slice(id.to_string().as_bytes());
        }
        None => bytes.push(0),
    }
    bytes.extend_from_slice(actor.to_string().as_bytes());
    match expected {
        ExpectedBase::Initial => bytes.push(0),
        ExpectedBase::Revision(id) => {
            bytes.push(1);
            bytes.extend_from_slice(id.to_string().as_bytes());
        }
    }
    Sha256::digest(&bytes).map_err(|_| integrity("drawing request digest failed"))
}

/// Converts a document sequence into the host's exact expected-base token.
pub fn expected_base(info: Option<&RevisionInfo>) -> Result<ExpectedBase, PlateError> {
    info.map_or(Ok(ExpectedBase::Initial), |info| {
        Ok(ExpectedBase::Revision(host_uuid(
            &info.revision().to_string(),
        )?))
    })
}

/// Parses a document's stable drawing identity from a canonical UUIDv7 string.
pub fn drawing_id(text: &str) -> Result<DrawingId, PlateError> {
    DrawingId::parse(text).map_err(|_| bad("drawing id is malformed"))
}
