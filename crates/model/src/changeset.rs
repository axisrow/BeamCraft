// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! C01 changeset: the unit a workspace sends for publication (architecture sections 13 and 14).
//!
//! A changeset names its base revision, the operations it carries (in draft order), its claimed
//! author, its semantic dependencies (read and write footprints), the complete layer set of the
//! resulting revision with each layer's base digest (the USD delta), and the immutable blobs a
//! receiver must hold. It is a message about a USD state, never a second persisted model: the
//! layers themselves stay USD blobs addressed by digest.
//!
//! # Encoding and identity
//!
//! [`Changeset::encode`] writes one canonical binary form (versioned, length-prefixed, every list
//! sorted and free of duplicates). [`Changeset::decode`] accepts only that form: it is bounded
//! (at most [`MAX_CHANGESET_BYTES`]), rejects trailing bytes, unknown tags and non-canonical
//! order, and never panics. [`Changeset::digest`] is the SHA-256 of the canonical bytes, so two
//! changesets are the same payload exactly when their digests are equal.
//!
//! # Idempotency and authority
//!
//! A changeset id names one payload forever. [`Changeset::admit`] decides a receipt from what the
//! receiver proved: the claimed actor must equal the actor of the trusted caller context
//! (`PERMISSION_DENIED` otherwise; the author string in the payload grants nothing), an id
//! already bound to the same digest is a replay, and an id bound to another digest is
//! `OP_ID_REUSE`. The binding itself is durable host state, not part of this crate.

use crate::command::CommandError;
use crate::digest::{ContentDigest, Sha256};
use crate::error::ErrorCode;
use crate::id::{ActorId, ElementId, OpId, ProjectId, RevisionId, Uuid7, WorkspaceId};
use crate::limits::MAX_FOOTPRINT_ENTRIES;

/// Largest canonical encoding a receiver decodes.
pub const MAX_CHANGESET_BYTES: usize = 1_048_576;
/// Most operations in one changeset.
pub const MAX_OPERATIONS: usize = 4096;
/// Most layers of the resulting revision.
pub const MAX_LAYERS: usize = 16;
/// Most immutable blobs a changeset requires.
pub const MAX_ASSETS: usize = 256;
/// Largest layer or blob a changeset may name (the host's 32 MiB blob cap).
pub const MAX_ASSET_BYTES: u64 = 32 * 1024 * 1024;
/// Longest layer name.
pub const MAX_LAYER_NAME_BYTES: usize = 64;

const MAGIC: &[u8] = b"BeamCraft/C01/Changeset/v1\0";
const MAGIC_WITH_MARK: &[u8] = b"BeamCraft/C01/Changeset/v2\0";

/// One operation the changeset carries, in draft order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Operation {
    /// The draft operation.
    pub op_id: OpId,
    /// Content identity of its accepted draft result (the draft revision's manifest digest).
    pub result: ContentDigest,
}

/// What a dependency refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DependencyTarget {
    /// An owned element.
    Element(ElementId),
    /// An immutable definition.
    Definition(ContentDigest),
}

/// The F04 section 8 semantic groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DependencyGroup {
    /// The label.
    Label,
    /// The complete shape-input group.
    ShapeInputs,
    /// The rigid placement.
    Placement,
    /// The material reference.
    MaterialRef,
    /// Relationships to other elements or definitions.
    Relationship,
    /// Existence and identity.
    Lifecycle,
    /// Derived geometry, quantities and their signatures.
    Derived,
    /// Numbering mark and its approval state (A08/A09); added as tag 7 without retagging v1.
    Mark,
}

impl DependencyGroup {
    /// Every group, in tag order.
    pub const ALL: [Self; 8] = [
        Self::Label,
        Self::ShapeInputs,
        Self::Placement,
        Self::MaterialRef,
        Self::Relationship,
        Self::Lifecycle,
        Self::Derived,
        Self::Mark,
    ];

    /// The F04 token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Label => "label",
            Self::ShapeInputs => "shape_inputs",
            Self::Placement => "placement",
            Self::MaterialRef => "material_ref",
            Self::Relationship => "relationship",
            Self::Lifecycle => "lifecycle",
            Self::Derived => "derived",
            Self::Mark => "mark",
        }
    }

    const fn tag(self) -> u8 {
        match self {
            Self::Label => 0,
            Self::ShapeInputs => 1,
            Self::Placement => 2,
            Self::MaterialRef => 3,
            Self::Relationship => 4,
            Self::Lifecycle => 5,
            Self::Derived => 6,
            Self::Mark => 7,
        }
    }
}

/// Read or write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Access {
    /// The changeset's result depends on the entry.
    Read,
    /// The changeset changes the entry.
    Write,
}

/// One footprint entry of the whole changeset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Dependency {
    /// Element or definition.
    pub target: DependencyTarget,
    /// Semantic group.
    pub group: DependencyGroup,
    /// Read or write.
    pub access: Access,
}

/// The role of a layer in the revision, as in the host manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LayerRole {
    /// The root layer and other owned authored layers.
    Authored,
    /// Derived results.
    Derived,
    /// A foreign, read-only layer.
    External,
}

/// One layer of the resulting revision.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LayerDelta {
    /// Virtual layer name (`[a-z0-9._-]`, ends in `.usda`).
    pub name: String,
    /// Manifest role.
    pub role: LayerRole,
    /// Digest of the layer in the base revision; `None` for a layer the base does not have.
    pub base: Option<ContentDigest>,
    /// Digest of the layer in the resulting revision.
    pub digest: ContentDigest,
    /// Byte size of the resulting layer.
    pub size: u64,
}

impl LayerDelta {
    /// Whether the changeset changes this layer.
    pub fn changed(&self) -> bool {
        self.base != Some(self.digest)
    }
}

/// One immutable blob the receiver must hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Asset {
    /// Full content digest.
    pub digest: ContentDigest,
    /// Byte size.
    pub size: u64,
}

/// A complete changeset. Construct it, then call [`Self::validate`] or [`Self::encode`] (which
/// validates first).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changeset {
    /// Identity of the publication; the published revision's operation id.
    pub changeset_id: OpId,
    /// Project.
    pub project: ProjectId,
    /// The workspace that produced it.
    pub workspace: WorkspaceId,
    /// Claimed author; checked against the trusted caller context, never a credential.
    pub actor: ActorId,
    /// The accepted revision the changeset is based on.
    pub base_revision: RevisionId,
    /// Manifest digest of that revision.
    pub base_manifest: ContentDigest,
    /// Identity the resulting published revision takes.
    pub revision: RevisionId,
    /// Draft operations, in draft order.
    pub operations: Vec<Operation>,
    /// Conservative read and write footprint, sorted.
    pub dependencies: Vec<Dependency>,
    /// The complete layer set of the resulting revision, sorted by name.
    pub layers: Vec<LayerDelta>,
    /// Name of the root layer (one of `layers`).
    pub root_layer: String,
    /// Immutable blobs required, sorted by digest; every changed layer's digest is one of them.
    pub assets: Vec<Asset>,
}

/// The decision of [`Changeset::admit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Receipt {
    /// A new changeset id: validate against the head and publish.
    Apply,
    /// The id is already bound to this exact payload: report the recorded result, change
    /// nothing.
    Replay,
}

fn error(code: ErrorCode, field: &'static str, message: &'static str) -> CommandError {
    CommandError::new(code, field, message)
}

fn limit(field: &'static str) -> CommandError {
    error(ErrorCode::LimitExceeded, field, "changeset exceeds a cap")
}

fn layer_name_valid(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_LAYER_NAME_BYTES
        && name.ends_with(".usda")
        && !name.starts_with('.')
        && name.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        })
}

/// Strictly increasing order (sorted and free of duplicates).
fn strictly_sorted<T: Ord>(items: &[T]) -> bool {
    items.windows(2).all(|w| matches!(w, [a, b] if a < b))
}

impl Changeset {
    /// Admits the structure: caps, canonical order, unique operations, a root layer among the
    /// layers, at least one changed layer, every changed layer's bytes among the assets, sizes
    /// within the bundle budget, and a revision identity distinct from the base. Whether the base
    /// is the receiver's head and whether the blobs and layers verify are the receiver's
    /// decisions.
    pub fn validate(&self) -> Result<(), CommandError> {
        if self.operations.is_empty() {
            return Err(error(
                ErrorCode::InvalidArgument,
                "operations",
                "a changeset carries at least one operation",
            ));
        }
        if self.operations.len() > MAX_OPERATIONS {
            return Err(limit("operations"));
        }
        let mut ops: Vec<OpId> = self.operations.iter().map(|o| o.op_id).collect();
        ops.sort();
        if !strictly_sorted(&ops) {
            return Err(error(
                ErrorCode::DuplicateId,
                "operations",
                "an operation appears twice",
            ));
        }
        if self.dependencies.len() > MAX_FOOTPRINT_ENTRIES {
            return Err(limit("dependencies"));
        }
        if !strictly_sorted(&self.dependencies) {
            return Err(error(
                ErrorCode::MalformedCommand,
                "dependencies",
                "dependencies are not sorted and unique",
            ));
        }
        if self.layers.is_empty() || self.layers.len() > MAX_LAYERS {
            return Err(limit("layers"));
        }
        if self.layers.iter().any(|l| !layer_name_valid(&l.name)) {
            return Err(error(
                ErrorCode::InvalidArgument,
                "layers",
                "layer name is not a plain lowercase .usda name",
            ));
        }
        let names: Vec<&str> = self.layers.iter().map(|l| l.name.as_str()).collect();
        if !strictly_sorted(&names) {
            return Err(error(
                ErrorCode::MalformedCommand,
                "layers",
                "layers are not sorted and unique by name",
            ));
        }
        if !names.contains(&self.root_layer.as_str()) {
            return Err(error(
                ErrorCode::InvalidArgument,
                "root_layer",
                "the root layer is not one of the layers",
            ));
        }
        if self.assets.len() > MAX_ASSETS {
            return Err(limit("assets"));
        }
        if !strictly_sorted(&self.assets) {
            return Err(error(
                ErrorCode::MalformedCommand,
                "assets",
                "assets are not sorted and unique",
            ));
        }
        let mut digests: Vec<ContentDigest> = self.assets.iter().map(|a| a.digest).collect();
        digests.sort();
        if !strictly_sorted(&digests) {
            return Err(error(
                ErrorCode::MalformedCommand,
                "assets",
                "an asset digest appears with two sizes",
            ));
        }
        if self.layers.iter().any(|l| l.size > MAX_ASSET_BYTES)
            || self.assets.iter().any(|a| a.size > MAX_ASSET_BYTES)
        {
            return Err(limit("size"));
        }
        if !self.layers.iter().any(LayerDelta::changed) {
            return Err(error(
                ErrorCode::InvalidArgument,
                "layers",
                "a changeset changes at least one layer",
            ));
        }
        for layer in self.layers.iter().filter(|l| l.changed()) {
            let asset = Asset {
                digest: layer.digest,
                size: layer.size,
            };
            if self.assets.binary_search(&asset).is_err() {
                return Err(error(
                    ErrorCode::InvalidArgument,
                    "assets",
                    "a changed layer is not among the assets",
                ));
            }
        }
        if self.revision == self.base_revision {
            return Err(error(
                ErrorCode::DuplicateId,
                "revision",
                "the resulting revision reuses the base identity",
            ));
        }
        Ok(())
    }

    /// The canonical bytes of a validated changeset.
    pub fn encode(&self) -> Result<Vec<u8>, CommandError> {
        self.validate()?;
        let mut out = Vec::with_capacity(512);
        let magic = if self
            .dependencies
            .iter()
            .any(|d| d.group == DependencyGroup::Mark)
        {
            MAGIC_WITH_MARK
        } else {
            MAGIC
        };
        out.extend_from_slice(magic);
        for id in [
            self.changeset_id.uuid(),
            self.project.uuid(),
            self.workspace.uuid(),
            self.actor.uuid(),
            self.base_revision.uuid(),
            self.revision.uuid(),
        ] {
            out.extend_from_slice(id.as_bytes());
        }
        out.extend_from_slice(self.base_manifest.as_bytes());
        push_count(&mut out, self.operations.len())?;
        for op in &self.operations {
            out.extend_from_slice(op.op_id.as_bytes());
            out.extend_from_slice(op.result.as_bytes());
        }
        push_count(&mut out, self.dependencies.len())?;
        for d in &self.dependencies {
            match d.target {
                DependencyTarget::Element(id) => {
                    out.push(0);
                    out.extend_from_slice(id.as_bytes());
                }
                DependencyTarget::Definition(digest) => {
                    out.push(1);
                    out.extend_from_slice(digest.as_bytes());
                }
            }
            out.push(d.group.tag());
            out.push(match d.access {
                Access::Read => 0,
                Access::Write => 1,
            });
        }
        push_count(&mut out, self.layers.len())?;
        for l in &self.layers {
            push_name(&mut out, &l.name)?;
            out.push(match l.role {
                LayerRole::Authored => 0,
                LayerRole::Derived => 1,
                LayerRole::External => 2,
            });
            match l.base {
                Some(base) => {
                    out.push(1);
                    out.extend_from_slice(base.as_bytes());
                }
                None => out.push(0),
            }
            out.extend_from_slice(l.digest.as_bytes());
            out.extend_from_slice(&l.size.to_le_bytes());
        }
        push_name(&mut out, &self.root_layer)?;
        push_count(&mut out, self.assets.len())?;
        for a in &self.assets {
            out.extend_from_slice(a.digest.as_bytes());
            out.extend_from_slice(&a.size.to_le_bytes());
        }
        if out.len() > MAX_CHANGESET_BYTES {
            return Err(limit("changeset"));
        }
        Ok(out)
    }

    /// The content identity: SHA-256 of the canonical bytes.
    pub fn digest(&self) -> Result<ContentDigest, CommandError> {
        Sha256::digest(&self.encode()?).map_err(|_| limit("changeset"))
    }

    /// Decodes and validates canonical bytes. Anything else is `MALFORMED_COMMAND` (structure),
    /// `INVALID_ID` (identity bytes), `LIMIT_EXCEEDED` (caps) or a validation code; a decoded
    /// changeset always re-encodes to exactly the input.
    pub fn decode(bytes: &[u8]) -> Result<Self, CommandError> {
        if bytes.len() > MAX_CHANGESET_BYTES {
            return Err(limit("changeset"));
        }
        let mut r = Reader { bytes, at: 0 };
        let magic = r.take(MAGIC.len())?;
        if magic != MAGIC && magic != MAGIC_WITH_MARK {
            return Err(error(
                ErrorCode::UnsupportedVersion,
                "changeset",
                "not a supported changeset version",
            ));
        }
        let changeset_id = OpId::from_uuid(r.uuid()?);
        let project = ProjectId::from_uuid(r.uuid()?);
        let workspace = WorkspaceId::from_uuid(r.uuid()?);
        let actor = ActorId::from_uuid(r.uuid()?);
        let base_revision = RevisionId::from_uuid(r.uuid()?);
        let revision = RevisionId::from_uuid(r.uuid()?);
        let base_manifest = r.digest()?;
        let count = r.count(MAX_OPERATIONS)?;
        let mut operations = Vec::with_capacity(count);
        for _ in 0..count {
            operations.push(Operation {
                op_id: OpId::from_uuid(r.uuid()?),
                result: r.digest()?,
            });
        }
        let count = r.count(MAX_FOOTPRINT_ENTRIES)?;
        let mut dependencies = Vec::with_capacity(count);
        for _ in 0..count {
            let target = match r.byte()? {
                0 => DependencyTarget::Element(ElementId::from_uuid(r.uuid()?)),
                1 => DependencyTarget::Definition(r.digest()?),
                _ => return Err(malformed()),
            };
            let group = *DependencyGroup::ALL
                .get(usize::from(r.byte()?))
                .ok_or_else(malformed)?;
            let access = match r.byte()? {
                0 => Access::Read,
                1 => Access::Write,
                _ => return Err(malformed()),
            };
            dependencies.push(Dependency {
                target,
                group,
                access,
            });
        }
        let count = r.count(MAX_LAYERS)?;
        let mut layers = Vec::with_capacity(count);
        for _ in 0..count {
            let name = r.name()?;
            let role = match r.byte()? {
                0 => LayerRole::Authored,
                1 => LayerRole::Derived,
                2 => LayerRole::External,
                _ => return Err(malformed()),
            };
            let base = match r.byte()? {
                0 => None,
                1 => Some(r.digest()?),
                _ => return Err(malformed()),
            };
            layers.push(LayerDelta {
                name,
                role,
                base,
                digest: r.digest()?,
                size: r.u64()?,
            });
        }
        let root_layer = r.name()?;
        let count = r.count(MAX_ASSETS)?;
        let mut assets = Vec::with_capacity(count);
        for _ in 0..count {
            assets.push(Asset {
                digest: r.digest()?,
                size: r.u64()?,
            });
        }
        if r.at != bytes.len() {
            return Err(malformed());
        }
        let changeset = Self {
            changeset_id,
            project,
            workspace,
            actor,
            base_revision,
            base_manifest,
            revision,
            operations,
            dependencies,
            layers,
            root_layer,
            assets,
        };
        // Validation, then the canonical round trip: one payload has exactly one encoding.
        if changeset.encode()? != bytes {
            return Err(malformed());
        }
        Ok(changeset)
    }

    /// Decides a receipt (architecture section 13, F04 section 6 order: authority before
    /// replay). `trusted_actor` is the actor of the authenticated caller context; `recorded` is
    /// the digest the receiver has durably bound to this changeset id, if any.
    pub fn admit(
        &self,
        trusted_actor: &ActorId,
        recorded: Option<ContentDigest>,
    ) -> Result<Receipt, CommandError> {
        let digest = self.digest()?;
        if self.actor != *trusted_actor {
            return Err(error(
                ErrorCode::PermissionDenied,
                "actor",
                "the claimed actor is not the caller",
            ));
        }
        match recorded {
            None => Ok(Receipt::Apply),
            Some(seen) if seen == digest => Ok(Receipt::Replay),
            Some(_) => Err(error(
                ErrorCode::OpIdReuse,
                "changeset_id",
                "changeset id was received with different content",
            )),
        }
    }
}

fn malformed() -> CommandError {
    error(
        ErrorCode::MalformedCommand,
        "changeset",
        "changeset bytes are not canonical",
    )
}

fn push_count(out: &mut Vec<u8>, n: usize) -> Result<(), CommandError> {
    let n = u32::try_from(n).map_err(|_| limit("count"))?;
    out.extend_from_slice(&n.to_le_bytes());
    Ok(())
}

fn push_name(out: &mut Vec<u8>, name: &str) -> Result<(), CommandError> {
    let n = u8::try_from(name.len()).map_err(|_| limit("name"))?;
    out.push(n);
    out.extend_from_slice(name.as_bytes());
    Ok(())
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], CommandError> {
        let end = self.at.checked_add(n).ok_or_else(malformed)?;
        let slice = self.bytes.get(self.at..end).ok_or_else(malformed)?;
        self.at = end;
        Ok(slice)
    }
    fn byte(&mut self) -> Result<u8, CommandError> {
        self.take(1)?.first().copied().ok_or_else(malformed)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], CommandError> {
        self.take(N)?.try_into().map_err(|_| malformed())
    }
    fn uuid(&mut self) -> Result<Uuid7, CommandError> {
        Uuid7::from_bytes(self.array::<16>()?).map_err(|_| {
            error(
                ErrorCode::InvalidId,
                "changeset",
                "identity is not a UUIDv7",
            )
        })
    }
    fn digest(&mut self) -> Result<ContentDigest, CommandError> {
        Ok(ContentDigest::from_bytes(self.array::<32>()?))
    }
    fn u64(&mut self) -> Result<u64, CommandError> {
        Ok(u64::from_le_bytes(self.array::<8>()?))
    }
    fn count(&mut self, cap: usize) -> Result<usize, CommandError> {
        let n = usize::try_from(u32::from_le_bytes(self.array::<4>()?)).map_err(|_| malformed())?;
        if n > cap {
            return Err(limit("count"));
        }
        Ok(n)
    }
    fn name(&mut self) -> Result<String, CommandError> {
        let n = usize::from(self.byte()?);
        let bytes = self.take(n)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| malformed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid(n: u16) -> Uuid7 {
        Uuid7::parse(&format!("019a7bf0-0000-7000-8000-00000000{n:04x}")).unwrap()
    }
    fn digest(text: &str) -> ContentDigest {
        Sha256::digest(text.as_bytes()).unwrap()
    }
    fn sample() -> Changeset {
        let authored = digest("authored v2");
        let derived = digest("derived v1");
        let mut assets = vec![
            Asset {
                digest: authored,
                size: 120,
            },
            Asset {
                digest: digest("geometry asset"),
                size: 9,
            },
        ];
        assets.sort();
        let mut dependencies = vec![
            Dependency {
                target: DependencyTarget::Element(ElementId::from_uuid(uuid(0x10))),
                group: DependencyGroup::Label,
                access: Access::Write,
            },
            Dependency {
                target: DependencyTarget::Element(ElementId::from_uuid(uuid(0x10))),
                group: DependencyGroup::Lifecycle,
                access: Access::Read,
            },
            Dependency {
                target: DependencyTarget::Definition(digest("material")),
                group: DependencyGroup::Relationship,
                access: Access::Read,
            },
        ];
        dependencies.sort();
        Changeset {
            changeset_id: OpId::from_uuid(uuid(1)),
            project: ProjectId::from_uuid(uuid(2)),
            workspace: WorkspaceId::from_uuid(uuid(3)),
            actor: ActorId::from_uuid(uuid(4)),
            base_revision: RevisionId::from_uuid(uuid(5)),
            base_manifest: digest("base manifest"),
            revision: RevisionId::from_uuid(uuid(6)),
            operations: vec![
                Operation {
                    op_id: OpId::from_uuid(uuid(8)),
                    result: digest("draft 2"),
                },
                Operation {
                    op_id: OpId::from_uuid(uuid(7)),
                    result: digest("draft 3"),
                },
            ],
            dependencies,
            layers: vec![
                LayerDelta {
                    name: "authored.usda".into(),
                    role: LayerRole::Authored,
                    base: Some(digest("authored v1")),
                    digest: authored,
                    size: 120,
                },
                LayerDelta {
                    name: "derived.usda".into(),
                    role: LayerRole::Derived,
                    base: Some(derived),
                    digest: derived,
                    size: 80,
                },
                LayerDelta {
                    name: "root.usda".into(),
                    role: LayerRole::Authored,
                    base: Some(digest("root")),
                    digest: digest("root"),
                    size: 60,
                },
            ],
            root_layer: "root.usda".into(),
            assets,
        }
    }

    #[test]
    fn legacy_payloads_stay_v1_and_numbering_dependencies_use_canonical_v2() {
        let legacy = sample();
        let bytes = legacy.encode().unwrap();
        assert!(bytes.starts_with(MAGIC));
        assert_eq!(Changeset::decode(&bytes).unwrap(), legacy);
        let mut numbered = legacy;
        numbered.dependencies.push(Dependency {
            target: DependencyTarget::Element(ElementId::from_uuid(uuid(0x10))),
            group: DependencyGroup::Mark,
            access: Access::Write,
        });
        numbered.dependencies.sort();
        let marked = numbered.encode().unwrap();
        assert!(marked.starts_with(MAGIC_WITH_MARK));
        assert_eq!(Changeset::decode(&marked).unwrap(), numbered);
        let mut wrong = marked.clone();
        wrong.get_mut(..MAGIC.len()).unwrap().copy_from_slice(MAGIC);
        assert!(
            Changeset::decode(&wrong).is_err(),
            "v1 cannot silently acquire a new group tag"
        );
        let mut redundant = bytes;
        redundant
            .get_mut(..MAGIC.len())
            .unwrap()
            .copy_from_slice(MAGIC_WITH_MARK);
        assert!(
            Changeset::decode(&redundant).is_err(),
            "one payload must have exactly one canonical encoding"
        );
    }

    #[test]
    fn the_canonical_form_round_trips_and_names_the_payload() {
        let c = sample();
        let bytes = c.encode().unwrap();
        assert_eq!(Changeset::decode(&bytes).unwrap(), c);
        assert_eq!(Sha256::digest(&bytes).unwrap(), c.digest().unwrap());
        // Operation order is draft order and part of the payload.
        let mut reordered = c.clone();
        reordered.operations.reverse();
        assert_ne!(reordered.digest().unwrap(), c.digest().unwrap());
        // Every field takes part in the identity, including the claimed actor.
        let mut other = c.clone();
        other.actor = ActorId::from_uuid(uuid(9));
        assert_ne!(other.digest().unwrap(), c.digest().unwrap());
    }

    #[test]
    fn same_payload_replays_other_payload_is_reuse_and_the_author_string_grants_nothing() {
        let c = sample();
        let me = c.actor;
        let digest = c.digest().unwrap();
        assert_eq!(c.admit(&me, None).unwrap(), Receipt::Apply);
        assert_eq!(c.admit(&me, Some(digest)).unwrap(), Receipt::Replay);
        let mut changed = c.clone();
        changed.revision = RevisionId::from_uuid(uuid(0x20));
        assert_eq!(
            changed.admit(&me, Some(digest)).unwrap_err().code,
            ErrorCode::OpIdReuse
        );
        // Authority is decided before replay: a mismatched caller learns nothing about the id.
        let stranger = ActorId::from_uuid(uuid(0x30));
        for recorded in [None, Some(digest)] {
            assert_eq!(
                c.admit(&stranger, recorded).unwrap_err().code,
                ErrorCode::PermissionDenied
            );
        }
    }

    #[test]
    fn structural_rules_are_enforced() {
        let code = |c: Changeset| c.validate().unwrap_err().code;
        let mut c = sample();
        c.operations.clear();
        assert_eq!(code(c), ErrorCode::InvalidArgument);
        let mut c = sample();
        c.operations[1].op_id = c.operations[0].op_id;
        assert_eq!(code(c), ErrorCode::DuplicateId);
        let mut c = sample();
        c.dependencies.reverse();
        assert_eq!(code(c), ErrorCode::MalformedCommand);
        let mut c = sample();
        c.layers.swap(0, 1);
        assert_eq!(code(c), ErrorCode::MalformedCommand);
        let mut c = sample();
        c.layers[0].name = "../authored.usda".into();
        assert_eq!(code(c), ErrorCode::InvalidArgument);
        let mut c = sample();
        c.root_layer = "missing.usda".into();
        assert_eq!(code(c), ErrorCode::InvalidArgument);
        let mut c = sample();
        c.layers[0].base = Some(c.layers[0].digest);
        assert_eq!(code(c), ErrorCode::InvalidArgument);
        let mut c = sample();
        c.assets.retain(|a| a.size != 120);
        assert_eq!(code(c), ErrorCode::InvalidArgument);
        let mut c = sample();
        c.layers[1].size = MAX_ASSET_BYTES + 1;
        assert_eq!(code(c), ErrorCode::LimitExceeded);
        let mut c = sample();
        c.revision = c.base_revision;
        assert_eq!(code(c), ErrorCode::DuplicateId);
    }

    #[test]
    fn decoding_never_panics_and_accepts_only_the_canonical_bytes() {
        let bytes = sample().encode().unwrap();
        // Every proper prefix, every single flipped byte and a trailing byte are refused or
        // decode to something that re-encodes to exactly those bytes.
        for n in 0..bytes.len() {
            assert!(Changeset::decode(&bytes[..n]).is_err());
        }
        for i in 0..bytes.len() {
            let mut flipped = bytes.clone();
            flipped[i] ^= 0x41;
            if let Ok(c) = Changeset::decode(&flipped) {
                assert_eq!(c.encode().unwrap(), flipped);
            }
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert_eq!(
            Changeset::decode(&longer).unwrap_err().code,
            ErrorCode::MalformedCommand
        );
        assert_eq!(
            Changeset::decode(&vec![0; MAX_CHANGESET_BYTES + 1])
                .unwrap_err()
                .code,
            ErrorCode::LimitExceeded
        );
        // A huge count is a cap violation before any allocation.
        let mut counted = MAGIC.to_vec();
        counted.extend_from_slice(&bytes[MAGIC.len()..MAGIC.len() + 6 * 16 + 32]);
        counted.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            Changeset::decode(&counted).unwrap_err().code,
            ErrorCode::LimitExceeded
        );
    }
}
