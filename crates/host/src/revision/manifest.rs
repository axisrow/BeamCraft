// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Canonical revision manifest: a closed, ordered text format with a strict bounded parser.
//!
//! ```text
//! beamcraft-revision-manifest 1
//! project <uuid7>
//! workspace <uuid7>
//! revision <uuid7>
//! op <uuid7>
//! sequence <decimal>
//! workspace-generation <decimal>
//! base none | base <uuid7> <manifest sha256>
//! schema <sha256>
//! root <layer name>
//! index none | index <structure sha256> <prim count>
//! layers <count>
//! layer <role> <name> <sha256> <size>      (count lines, strictly ascending by name)
//! ```
//!
//! Every line ends with a single `\n`; there is no other whitespace, no optional line and no
//! trailing byte. A parsed manifest is re-encoded and must reproduce the input exactly, so every
//! stored manifest has exactly one valid byte form.
//!
//! Version 2 uses `beamcraft-revision-manifest 2` and exactly one additional
//! `evidence <sha256> <size>` line between `index` and `layers`. Its opaque, capped blob is
//! operation evidence, never a USD layer. A revision without this pin retains byte-for-byte
//! version 1 encoding. New readers accept both; older readers refuse version 2 explicitly.

use std::fmt::Write as _;

use beamcraft_model::digest::{ContentDigest, Sha256};

use crate::storage::MAX_BLOB_BYTES;

use super::{
    MAX_ENTRIES, MAX_MANIFEST_BYTES, MAX_NAME_BYTES, MAX_REVISION_BYTES, MAX_REVISIONS,
    RevisionError, Uuid7,
};

const MAGIC: &str = "beamcraft-revision-manifest 1";
const MAGIC_WITH_EVIDENCE: &str = "beamcraft-revision-manifest 2";

/// An opaque operation-evidence blob's complete identity and bounded size. It is not a USD
/// layer or authority for its contents; the application must decode and verify its semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvidencePin {
    digest: ContentDigest,
    size: u64,
}

impl EvidencePin {
    /// Admits a pin up to the operation-evidence cap. Commit/read verify its actual bytes.
    pub fn new(digest: ContentDigest, size: u64) -> Result<Self, RevisionError> {
        if size > super::MAX_OPERATION_EVIDENCE_BYTES {
            return Err(RevisionError::LimitExceeded);
        }
        Ok(Self { digest, size })
    }

    /// Full SHA-256 of the opaque blob.
    pub const fn digest(self) -> ContentDigest {
        self.digest
    }

    /// Declared byte length.
    pub const fn size(self) -> u64 {
        self.size
    }
}

/// Role of one layer in a revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LayerRole {
    /// Owned authored model layer (U02 authored layer or the root that composes it).
    Authored,
    /// Owned derived layer (computed geometry and quantities).
    Derived,
    /// Foreign or read-only asset whose original bytes the revision preserves verbatim.
    External,
}

impl LayerRole {
    /// Stable manifest spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Authored => "authored",
            Self::Derived => "derived",
            Self::External => "external",
        }
    }

    /// Parses the stable manifest spelling.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "authored" => Some(Self::Authored),
            "derived" => Some(Self::Derived),
            "external" => Some(Self::External),
            _ => None,
        }
    }
}

/// One immutable layer blob of a revision: a virtual name, a role, the full digest and the size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerEntry {
    name: String,
    role: LayerRole,
    digest: ContentDigest,
    size: u64,
}

/// True for a canonical virtual layer identifier: 1 to [`MAX_NAME_BYTES`] ASCII bytes of
/// `[A-Za-z0-9._-]` segments joined by single `/`, with no empty, `.` or `..` segment.
/// A name is never a filesystem path; it identifies a layer inside the revision.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && name.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && segment
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        })
}

impl LayerEntry {
    /// Validates the name and the size cap. The digest and size are claims until a commit or
    /// read verifies them against the stored blob.
    pub fn new(
        name: &str,
        role: LayerRole,
        digest: ContentDigest,
        size: u64,
    ) -> Result<Self, RevisionError> {
        if !valid_name(name) {
            return Err(RevisionError::InvalidArgument);
        }
        if size > MAX_BLOB_BYTES as u64 {
            return Err(RevisionError::LimitExceeded);
        }
        Ok(Self {
            name: name.to_owned(),
            role,
            digest,
            size,
        })
    }

    /// Computes the digest and size of exact layer bytes.
    pub fn from_bytes(name: &str, role: LayerRole, bytes: &[u8]) -> Result<Self, RevisionError> {
        let digest = Sha256::digest(bytes).map_err(|_| RevisionError::LimitExceeded)?;
        let size = u64::try_from(bytes.len()).map_err(|_| RevisionError::LimitExceeded)?;
        Self::new(name, role, digest, size)
    }

    /// Virtual layer name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Layer role.
    pub const fn role(&self) -> LayerRole {
        self.role
    }

    /// Full SHA-256 of the layer bytes.
    pub const fn digest(&self) -> ContentDigest {
        self.digest
    }

    /// Declared size in bytes.
    pub const fn size(&self) -> u64 {
        self.size
    }
}

/// Everything a caller decides about one revision. Layer order is irrelevant: the commit sorts
/// the entries, so 20 reordered saves of the same layers yield byte-identical manifests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionDraft {
    /// Project the revision belongs to.
    pub project: Uuid7,
    /// Workspace that published it.
    pub workspace: Uuid7,
    /// New immutable revision id; reusing it for different content is an integrity error.
    pub revision: Uuid7,
    /// Operation that created the revision; the retry key for idempotent commit.
    pub op: Uuid7,
    /// Workspace generation the layers were produced at. It is the generation a fresh reader
    /// pins when it reopens the revision.
    pub workspace_generation: u64,
    /// Identity digest of the authored schema (U02 `SchemaIdentity::digest`).
    pub schema: ContentDigest,
    /// Name of the entry that is the composition root.
    pub root_layer: String,
    /// The complete layer set, in any order.
    pub entries: Vec<LayerEntry>,
}

/// The base revision a manifest builds on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseRef {
    /// Revision id of the base.
    pub revision: Uuid7,
    /// Digest of the base's manifest blob.
    pub manifest: ContentDigest,
}

/// Verified U03 index evidence recorded in a manifest: the structure digest and prim count.
/// The index is a cache; it is recorded so a reader can compare a rebuilt index with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexRecord {
    /// `IndexToken::structure_digest`.
    pub structure: ContentDigest,
    /// Number of prims indexed.
    pub prims: u64,
}

/// The caller-decided part of a manifest: everything except sequence, base and index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Content {
    project: Uuid7,
    workspace: Uuid7,
    revision: Uuid7,
    op: Uuid7,
    workspace_generation: u64,
    schema: ContentDigest,
    root_layer: String,
    entries: Vec<LayerEntry>,
    operation_evidence: Option<EvidencePin>,
}

impl Content {
    /// Validates and normalizes a draft: at most [`MAX_ENTRIES`] entries, no duplicate name, at
    /// least one authored and one derived layer, a root layer that is an entry, and a bounded
    /// total size. Entries are sorted by name.
    pub fn from_draft(draft: &RevisionDraft) -> Result<Self, RevisionError> {
        Self::from_draft_with_evidence(draft, None)
    }

    /// Normalizes the same draft with an optional immutable operation-evidence pin. The pin
    /// participates in the content fingerprint and every operation/revision replay comparison.
    pub fn from_draft_with_evidence(
        draft: &RevisionDraft,
        operation_evidence: Option<EvidencePin>,
    ) -> Result<Self, RevisionError> {
        if draft.entries.len() > MAX_ENTRIES {
            return Err(RevisionError::LimitExceeded);
        }
        let mut entries = draft.entries.clone();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        if entries
            .windows(2)
            .any(|w| matches!(w, [a, b] if a.name == b.name))
        {
            return Err(RevisionError::DuplicateEntry);
        }
        let content = Self {
            project: draft.project.clone(),
            workspace: draft.workspace.clone(),
            revision: draft.revision.clone(),
            op: draft.op.clone(),
            workspace_generation: draft.workspace_generation,
            schema: draft.schema,
            root_layer: draft.root_layer.clone(),
            entries,
            operation_evidence,
        };
        content.check()?;
        Ok(content)
    }

    fn check(&self) -> Result<(), RevisionError> {
        if !valid_name(&self.root_layer)
            || !self.entries.iter().any(|e| e.name == self.root_layer)
            || !self.entries.iter().any(|e| e.role == LayerRole::Authored)
            || !self.entries.iter().any(|e| e.role == LayerRole::Derived)
        {
            return Err(RevisionError::InvalidArgument);
        }
        let mut total = 0u64;
        for entry in &self.entries {
            total = total
                .checked_add(entry.size)
                .filter(|t| *t <= MAX_REVISION_BYTES)
                .ok_or(RevisionError::LimitExceeded)?;
        }
        Ok(())
    }

    /// Identity of the caller-decided content alone: the SHA-256 of the canonical manifest
    /// encoding with sequence 1, no base and no index. The same content has the same
    /// fingerprint whatever it is later stacked on, so the U07 ledger uses it to bind an op id
    /// to what that operation was asked to commit.
    pub fn fingerprint(&self) -> Result<ContentDigest, RevisionError> {
        let bytes = Manifest::assemble(self.clone(), 1, None, None)?.encode();
        Sha256::digest(&bytes).map_err(|_| RevisionError::LimitExceeded)
    }

    /// Project id.
    pub fn project(&self) -> &Uuid7 {
        &self.project
    }

    /// Publishing workspace id.
    pub fn workspace(&self) -> &Uuid7 {
        &self.workspace
    }

    /// Revision id.
    pub fn revision(&self) -> &Uuid7 {
        &self.revision
    }

    /// Creating operation id.
    pub fn op(&self) -> &Uuid7 {
        &self.op
    }

    /// Workspace generation of the layers.
    pub const fn workspace_generation(&self) -> u64 {
        self.workspace_generation
    }

    /// Schema identity digest.
    pub const fn schema(&self) -> ContentDigest {
        self.schema
    }

    /// Composition root layer name.
    pub fn root_layer(&self) -> &str {
        &self.root_layer
    }

    /// Entries sorted strictly ascending by name.
    pub fn entries(&self) -> &[LayerEntry] {
        &self.entries
    }

    /// Opaque operation evidence; absent in legacy revisions and commits without evidence.
    pub const fn operation_evidence(&self) -> Option<EvidencePin> {
        self.operation_evidence
    }
}

/// One immutable, canonical revision manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    content: Content,
    sequence: u64,
    base: Option<BaseRef>,
    index: Option<IndexRecord>,
}

impl Manifest {
    /// Assembles a manifest. Sequence 1 has no base and every later sequence has one; the
    /// sequence is capped at [`MAX_REVISIONS`].
    pub fn assemble(
        content: Content,
        sequence: u64,
        base: Option<BaseRef>,
        index: Option<IndexRecord>,
    ) -> Result<Self, RevisionError> {
        if sequence == 0 || sequence > MAX_REVISIONS {
            return Err(RevisionError::LimitExceeded);
        }
        if base.is_none() != (sequence == 1)
            || base
                .as_ref()
                .is_some_and(|b| b.revision == content.revision)
        {
            return Err(RevisionError::InvalidArgument);
        }
        Ok(Self {
            content,
            sequence,
            base,
            index,
        })
    }

    /// The caller-decided content.
    pub fn content(&self) -> &Content {
        &self.content
    }

    /// Revision number, starting at 1.
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// The base revision, absent only for sequence 1.
    pub fn base(&self) -> Option<&BaseRef> {
        self.base.as_ref()
    }

    /// Verified U03 index evidence, absent when the commit had no content verifier.
    pub const fn index(&self) -> Option<IndexRecord> {
        self.index
    }

    /// Canonical bytes.
    pub fn encode(&self) -> Vec<u8> {
        let c = &self.content;
        let mut out = String::new();
        // Writing to a String cannot fail, so the results are intentionally ignored.
        let magic = if c.operation_evidence.is_some() {
            MAGIC_WITH_EVIDENCE
        } else {
            MAGIC
        };
        let _ = writeln!(out, "{magic}");
        let _ = writeln!(out, "project {}", c.project);
        let _ = writeln!(out, "workspace {}", c.workspace);
        let _ = writeln!(out, "revision {}", c.revision);
        let _ = writeln!(out, "op {}", c.op);
        let _ = writeln!(out, "sequence {}", self.sequence);
        let _ = writeln!(out, "workspace-generation {}", c.workspace_generation);
        match &self.base {
            Some(b) => {
                let _ = writeln!(out, "base {} {}", b.revision, b.manifest);
            }
            None => {
                let _ = writeln!(out, "base none");
            }
        }
        let _ = writeln!(out, "schema {}", c.schema);
        let _ = writeln!(out, "root {}", c.root_layer);
        match &self.index {
            Some(i) => {
                let _ = writeln!(out, "index {} {}", i.structure, i.prims);
            }
            None => {
                let _ = writeln!(out, "index none");
            }
        }
        if let Some(pin) = c.operation_evidence {
            let _ = writeln!(out, "evidence {} {}", pin.digest, pin.size);
        }
        let _ = writeln!(out, "layers {}", c.entries.len());
        for e in &c.entries {
            let _ = writeln!(
                out,
                "layer {} {} {} {}",
                e.role.as_str(),
                e.name,
                e.digest,
                e.size
            );
        }
        out.into_bytes()
    }

    /// SHA-256 of the canonical bytes: the manifest blob's address and the identity that U01
    /// carries as `Identity::revision`.
    pub fn digest(&self) -> Result<ContentDigest, RevisionError> {
        Sha256::digest(&self.encode()).map_err(|_| RevisionError::LimitExceeded)
    }

    /// Strict parser. Any deviation from the canonical form is
    /// [`RevisionError::MalformedManifest`]; the input is never indexed by position.
    pub fn parse(bytes: &[u8]) -> Result<Self, RevisionError> {
        let bad = RevisionError::MalformedManifest;
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(bad);
        }
        let text = std::str::from_utf8(bytes).map_err(|_| bad)?;
        let body = text.strip_suffix('\n').ok_or(bad)?;
        let mut lines = body.split('\n');
        let has_evidence = match lines.next() {
            Some(MAGIC) => false,
            Some(MAGIC_WITH_EVIDENCE) => true,
            _ => return Err(bad),
        };
        let id = |value: &str| Uuid7::parse(value).map_err(|_| bad);
        let digest = |value: &str| value.parse::<ContentDigest>().map_err(|_| bad);
        let project = id(field(&mut lines, "project")?)?;
        let workspace = id(field(&mut lines, "workspace")?)?;
        let revision = id(field(&mut lines, "revision")?)?;
        let op = id(field(&mut lines, "op")?)?;
        let sequence = decimal(field(&mut lines, "sequence")?)?;
        let workspace_generation = decimal(field(&mut lines, "workspace-generation")?)?;
        let base = match field(&mut lines, "base")? {
            "none" => None,
            value => {
                let mut parts = value.split(' ');
                let (Some(r), Some(m), None) = (parts.next(), parts.next(), parts.next()) else {
                    return Err(bad);
                };
                Some(BaseRef {
                    revision: id(r)?,
                    manifest: digest(m)?,
                })
            }
        };
        let schema = digest(field(&mut lines, "schema")?)?;
        let root_layer = field(&mut lines, "root")?.to_owned();
        let index = match field(&mut lines, "index")? {
            "none" => None,
            value => {
                let mut parts = value.split(' ');
                let (Some(s), Some(p), None) = (parts.next(), parts.next(), parts.next()) else {
                    return Err(bad);
                };
                Some(IndexRecord {
                    structure: digest(s)?,
                    prims: decimal(p)?,
                })
            }
        };
        let operation_evidence = if has_evidence {
            let mut parts = field(&mut lines, "evidence")?.split(' ');
            let (Some(d), Some(size), None) = (parts.next(), parts.next(), parts.next()) else {
                return Err(bad);
            };
            Some(EvidencePin::new(digest(d)?, decimal(size)?).map_err(|_| bad)?)
        } else {
            None
        };
        let count = usize::try_from(decimal(field(&mut lines, "layers")?)?).map_err(|_| bad)?;
        if count > MAX_ENTRIES {
            return Err(bad);
        }
        let mut entries: Vec<LayerEntry> = Vec::with_capacity(count);
        for _ in 0..count {
            let mut parts = field(&mut lines, "layer")?.split(' ');
            let (Some(role), Some(name), Some(d), Some(size), None) = (
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
            ) else {
                return Err(bad);
            };
            let entry = LayerEntry::new(
                name,
                LayerRole::parse(role).ok_or(bad)?,
                digest(d)?,
                decimal(size)?,
            )
            .map_err(|_| bad)?;
            // Strictly ascending names: ordered and duplicate-free.
            if entries.last().is_some_and(|last| last.name >= entry.name) {
                return Err(bad);
            }
            entries.push(entry);
        }
        if lines.next().is_some() {
            return Err(bad);
        }
        let content = Content {
            project,
            workspace,
            revision,
            op,
            workspace_generation,
            schema,
            root_layer,
            entries,
            operation_evidence,
        };
        content.check().map_err(|_| bad)?;
        let manifest = Self::assemble(content, sequence, base, index).map_err(|_| bad)?;
        if manifest.encode() != bytes {
            return Err(bad);
        }
        Ok(manifest)
    }
}

/// Takes the next line, which must be `<key> <value>`, and returns the value.
fn field<'a>(
    lines: &mut impl Iterator<Item = &'a str>,
    key: &str,
) -> Result<&'a str, RevisionError> {
    lines
        .next()
        .and_then(|line| line.strip_prefix(key))
        .and_then(|rest| rest.strip_prefix(' '))
        .ok_or(RevisionError::MalformedManifest)
}

/// Canonical unsigned decimal: digits only, no sign, no leading zero except `0`, at most u64.
fn decimal(text: &str) -> Result<u64, RevisionError> {
    let canonical = !text.is_empty()
        && text.len() <= 20
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    if !canonical {
        return Err(RevisionError::MalformedManifest);
    }
    text.parse().map_err(|_| RevisionError::MalformedManifest)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    fn id(n: u32) -> Uuid7 {
        Uuid7::parse(&format!("0199e7a0-0000-7000-8000-{n:012x}")).unwrap()
    }
    fn entry(name: &str, role: LayerRole, body: &str) -> LayerEntry {
        LayerEntry::from_bytes(name, role, body.as_bytes()).unwrap()
    }
    fn draft() -> RevisionDraft {
        RevisionDraft {
            project: id(1),
            workspace: id(2),
            revision: id(3),
            op: id(4),
            workspace_generation: 7,
            schema: Sha256::digest(b"schema").unwrap(),
            root_layer: "root.usda".into(),
            entries: vec![
                entry("root.usda", LayerRole::Authored, "root"),
                entry("authored.usda", LayerRole::Authored, "a"),
                entry("derived.usda", LayerRole::Derived, "d"),
                entry("ext/weak.usda", LayerRole::External, "w"),
            ],
        }
    }
    fn manifest() -> Manifest {
        let base = BaseRef {
            revision: id(9),
            manifest: Sha256::digest(b"base").unwrap(),
        };
        Manifest::assemble(
            Content::from_draft(&draft()).unwrap(),
            2,
            Some(base),
            Some(IndexRecord {
                structure: Sha256::digest(b"index").unwrap(),
                prims: 12,
            }),
        )
        .unwrap()
    }

    #[test]
    fn round_trip_is_canonical_and_sorted() {
        let m = manifest();
        let bytes = m.encode();
        assert_eq!(Manifest::parse(&bytes).unwrap(), m);
        let names: Vec<_> = m.content().entries().iter().map(LayerEntry::name).collect();
        assert_eq!(
            names,
            [
                "authored.usda",
                "derived.usda",
                "ext/weak.usda",
                "root.usda"
            ]
        );
        assert!(bytes.starts_with(b"beamcraft-revision-manifest 1\nproject "));
    }

    #[test]
    fn every_entry_order_gives_identical_bytes() {
        let reference = manifest().encode();
        let mut d = draft();
        for round in 0..20usize {
            let len = d.entries.len();
            d.entries.rotate_left(round % len + 1);
            if round % 2 == 1 {
                d.entries.reverse();
            }
            let m = Manifest::assemble(
                Content::from_draft(&d).unwrap(),
                2,
                manifest().base().cloned(),
                manifest().index(),
            )
            .unwrap();
            assert_eq!(m.encode(), reference, "round {round}");
        }
    }

    #[test]
    fn draft_rules_each_have_a_negative_case() {
        let mut d = draft();
        d.entries.push(entry("root.usda", LayerRole::Derived, "x"));
        assert_eq!(Content::from_draft(&d), Err(RevisionError::DuplicateEntry));
        let mut d = draft();
        d.root_layer = "missing.usda".into();
        assert_eq!(Content::from_draft(&d), Err(RevisionError::InvalidArgument));
        let mut d = draft();
        d.entries.retain(|e| e.role() != LayerRole::Derived);
        assert_eq!(Content::from_draft(&d), Err(RevisionError::InvalidArgument));
        let mut d = draft();
        d.entries.retain(|e| e.role() != LayerRole::Authored);
        assert_eq!(Content::from_draft(&d), Err(RevisionError::InvalidArgument));
        let mut d = draft();
        d.entries = (0..=MAX_ENTRIES)
            .map(|i| entry(&format!("l{i}.usda"), LayerRole::External, "x"))
            .collect();
        assert_eq!(Content::from_draft(&d), Err(RevisionError::LimitExceeded));
        // Nine maximal blobs exceed the revision byte budget even though each is legal.
        let mut d = draft();
        d.entries = (0..9)
            .map(|i| {
                let role = if i == 0 {
                    LayerRole::Derived
                } else {
                    LayerRole::Authored
                };
                LayerEntry::new(
                    &format!("l{i}.usda"),
                    role,
                    Sha256::digest(b"x").unwrap(),
                    MAX_BLOB_BYTES as u64,
                )
                .unwrap()
            })
            .collect();
        d.root_layer = "l1.usda".into();
        assert_eq!(Content::from_draft(&d), Err(RevisionError::LimitExceeded));
        let digest = Sha256::digest(b"").unwrap();
        assert_eq!(
            LayerEntry::new(
                "a.usda",
                LayerRole::Authored,
                digest,
                MAX_BLOB_BYTES as u64 + 1
            ),
            Err(RevisionError::LimitExceeded)
        );
        for name in [
            "",
            "/a",
            "a/",
            "a//b",
            "./a",
            "a/../b",
            "..",
            "a b",
            "a\\b",
            "C:/a",
            "a\n",
            "é.usda",
            &"x".repeat(MAX_NAME_BYTES + 1),
        ] {
            assert_eq!(
                LayerEntry::new(name, LayerRole::Authored, digest, 0),
                Err(RevisionError::InvalidArgument),
                "{name:?}"
            );
        }
        for bad in [
            "",
            "0199e7a0",
            "0199E7A0-0000-7000-8000-000000000001",
            "0199e7a0-0000-4000-8000-000000000001",
        ] {
            assert_eq!(Uuid7::parse(bad), Err(RevisionError::InvalidId));
        }
    }

    #[test]
    fn assemble_enforces_sequence_and_base_rules() {
        let c = Content::from_draft(&draft()).unwrap();
        let base = manifest().base().cloned();
        let assemble = |seq, base| Manifest::assemble(c.clone(), seq, base, None);
        assert_eq!(assemble(0, None), Err(RevisionError::LimitExceeded));
        assert_eq!(
            assemble(MAX_REVISIONS + 1, base.clone()),
            Err(RevisionError::LimitExceeded)
        );
        assert_eq!(assemble(1, base), Err(RevisionError::InvalidArgument));
        assert_eq!(assemble(2, None), Err(RevisionError::InvalidArgument));
        let own = BaseRef {
            revision: c.revision().clone(),
            manifest: Sha256::digest(b"b").unwrap(),
        };
        assert_eq!(assemble(2, Some(own)), Err(RevisionError::InvalidArgument));
    }

    fn mutate(from: &str, to: &str) -> Vec<u8> {
        let text = String::from_utf8(manifest().encode()).unwrap();
        assert!(text.contains(from), "{from}");
        text.replacen(from, to, 1).into_bytes()
    }

    #[test]
    fn parser_rejects_every_non_canonical_form() {
        let good = manifest().encode();
        assert!(Manifest::parse(&good).is_ok());
        let text = String::from_utf8(good.clone()).unwrap();
        let mut cases: Vec<Vec<u8>> = vec![
            Vec::new(),
            good[..good.len() - 1].to_vec(),
            [good.clone(), b"\n".to_vec()].concat(),
            [good.clone(), b"extra\n".to_vec()].concat(),
            good[..good.len() / 2].to_vec(),
            text.replace('\n', "\r\n").into_bytes(),
            text.replacen("manifest 1", "manifest 2", 1).into_bytes(),
            mutate("sequence 2", "sequence 02"),
            mutate("sequence 2", "sequence +2"),
            mutate("sequence 2", "sequence 1"),
            mutate("sequence 2", "sequence 0"),
            mutate("sequence 2", "sequence 18446744073709551616"),
            mutate("workspace-generation 7", "workspace-generation  7"),
            mutate("op ", "op  "),
            mutate("index ", "index  "),
            mutate("layers 4", "layers 3"),
            mutate("layers 4", "layers 5"),
            mutate("layers 4", "layers 65"),
            mutate(
                "layer authored authored.usda",
                "layer authored  authored.usda",
            ),
            mutate("layer authored authored.usda", "layer binary authored.usda"),
            mutate("root root.usda", "root ../root.usda"),
            mutate("root root.usda", "root other.usda"),
            mutate("base ", "base  "),
            mutate("schema ", "schema A"),
            vec![0xff, 0xfe, b'\n'],
            vec![b'x'; MAX_MANIFEST_BYTES + 1],
        ];
        // Unordered and duplicate entries, and out-of-order header lines.
        let lines: Vec<&str> = text.trim_end().split('\n').collect();
        let join = |l: &[&str]| format!("{}\n", l.join("\n")).into_bytes();
        let mut swapped = lines.clone();
        swapped.swap(12, 13);
        cases.push(join(&swapped));
        let mut repeated = lines.clone();
        repeated[13] = repeated[12];
        cases.push(join(&repeated));
        let mut reordered = lines.clone();
        reordered.swap(1, 2);
        cases.push(join(&reordered));
        for case in &cases {
            assert_eq!(
                Manifest::parse(case),
                Err(RevisionError::MalformedManifest),
                "{:?}",
                String::from_utf8_lossy(&case[..case.len().min(80)])
            );
        }
    }

    #[test]
    fn parser_applies_the_semantic_rules_too() {
        // Structurally valid text with no derived layer still violates the manifest rules.
        let text = String::from_utf8(manifest().encode()).unwrap();
        let no_derived = text
            .lines()
            .filter(|l| !l.starts_with("layer derived"))
            .map(|l| if l == "layers 4" { "layers 3" } else { l })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        assert_eq!(
            Manifest::parse(no_derived.as_bytes()),
            Err(RevisionError::MalformedManifest)
        );
    }
}
