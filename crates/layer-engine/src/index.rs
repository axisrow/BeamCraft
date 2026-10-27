// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! U03: reconstructible UUID and relationship index.
//!
//! The index is derived only from complete, pinned snapshot pages produced by this crate's
//! reader (never from upstream handles). It is a cache of the pinned USD state, not authority:
//! it is bound to one context, is rebuilt from scratch, and a stale, partial or forged index
//! cannot authorize a write or certify the absence of conflicts. A persisted index or a USD path
//! never becomes element identity; the full UUIDv7 is the key.

use crate::authored::{Footprint, Group};
use crate::preflight::charge;
use crate::*;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use beamcraft_model::prim::{DefinitionCategory, ElementCategory};
const MAX_LABEL_BYTES: usize = 256;
const MAX_PATH_BYTES: usize = 1024;
/// Total relationship targets one index may hold (the 1 MiB snapshot ceiling at 32 bytes each).
pub const MAX_LINKS: usize = 32_768;

fn invalid_id(message: &'static str) -> Diagnostic {
    fail(ErrorCode::InvalidId, message)
}
fn unavailable(message: &'static str) -> Diagnostic {
    fail(ErrorCode::DependencyUnavailable, message)
}
fn is_lower_hex(s: &str) -> bool {
    s.bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A full 128-bit UUIDv7 element identity. Equality, ordering and lookup use all 128 bits, so
/// elements created in the same millisecond are never merged or shortened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ElementId(u128);

impl ElementId {
    /// Parses the canonical lowercase `8-4-4-4-12` form with version 7 and the RFC variant.
    /// Short, truncated, upper-case, unhyphenated and padded text is `INVALID_ID`.
    pub fn parse(text: &str) -> Result<Self, Diagnostic> {
        if !uuid7(text) {
            return Err(invalid_id(
                "element identity is not a full canonical UUIDv7",
            ));
        }
        let mut value = 0u128;
        for byte in text.bytes().filter(|b| *b != b'-') {
            let nibble = char::from(byte)
                .to_digit(16)
                .ok_or_else(|| invalid_id("element identity is not a full canonical UUIDv7"))?;
            value = (value << 4) | u128::from(nibble);
        }
        Ok(Self(value))
    }
    /// Parses a canonical category path with 32 lowercase UUIDv7 digits; any other form is `INVALID_ID`.
    pub fn from_path(path: &str) -> Result<Self, Diagnostic> {
        let bad = || invalid_id("element path lacks a full 32-digit UUIDv7");
        let hex = ElementCategory::ALL
            .into_iter()
            .find_map(|category| path.strip_prefix(category.prefix()))
            .ok_or_else(bad)?;
        if hex.len() != 32 || !is_lower_hex(hex) {
            return Err(bad());
        }
        let part = |from: usize, to: usize| hex.get(from..to).ok_or_else(bad);
        Self::parse(&format!(
            "{}-{}-{}-{}-{}",
            part(0, 8)?,
            part(8, 12)?,
            part(12, 16)?,
            part(16, 20)?,
            part(20, 32)?
        ))
    }
    /// The complete 128-bit value.
    pub fn as_u128(&self) -> u128 {
        self.0
    }
    /// The canonical prim path of this element.
    pub fn path(&self, category: ElementCategory) -> String {
        format!("{}{:032x}", category.prefix(), self.0)
    }
}
impl fmt::Display for ElementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let v = self.0;
        write!(
            f,
            "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
            (v >> 96) as u32,
            ((v >> 80) & 0xffff) as u16,
            ((v >> 64) & 0xffff) as u16,
            ((v >> 48) & 0xffff) as u16,
            (v & 0xffff_ffff_ffff) as u64
        )
    }
}

/// Parses a footprint identity: a full element UUIDv7 or a full content digest.
pub(crate) fn footprint_element(identity: &str) -> Result<Option<ElementId>, Diagnostic> {
    if let Ok(id) = ElementId::parse(identity) {
        Ok(Some(id))
    } else if identity.parse::<ContentDigest>().is_ok() {
        Ok(None)
    } else {
        Err(invalid_id(
            "footprint identity is neither a full UUIDv7 nor a digest",
        ))
    }
}

/// `bim:isDeactivated` state (`DV-BASE-004`). A deactivated element keeps its identity and links.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    /// In play. Also the state of an element that authors no `bim:isDeactivated`.
    Active,
    /// Retained but out of play; links to it stay valid and are reported.
    Deactivated,
}

/// Ownership derived from every contributing composition site, the prim's ancestors and its
/// descendants. Unknown ownership is never `Owned`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ownership {
    /// Every contributing site of the element, its ancestors and descendants is host-owned.
    Owned,
    /// Some contributing site is foreign: read-only for every wrapper route.
    Foreign,
}

/// Whether the index covers the whole pinned snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completeness {
    /// Every page of the pinned snapshot was indexed.
    Complete,
    /// Only a prefix was indexed: absence proves nothing, and nothing is authorized or certified.
    Partial,
}

/// One indexed element, with composition provenance. The label never contributes to identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementEntry {
    /// Full identity.
    pub id: ElementId,
    /// Canonical composed prim path (derived from the ID, never the other way round).
    pub path: String,
    /// `bim:elementKind`, if authored.
    pub kind: Option<String>,
    /// `bim:name`, if authored.
    pub label: Option<String>,
    /// Lifecycle state.
    pub lifecycle: Lifecycle,
    /// Composed USD `active` state of the prim.
    pub usd_active: bool,
    /// Ownership verdict.
    pub ownership: Ownership,
    /// Every contributing site, strongest first.
    pub sources: Vec<Provenance>,
}

/// An immutable definition prim, keyed by its full content digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionEntry {
    /// Full digest from the `MAT_<64 hex>` or `PRF_<64 hex>` path.
    pub digest: ContentDigest,
    /// Composed prim path.
    pub path: String,
    /// Every contributing site, strongest first.
    pub sources: Vec<Provenance>,
}

/// Resolved target of one relationship entry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum LinkTarget {
    /// An indexed element.
    Element(ElementId),
    /// An indexed immutable definition.
    Definition(ContentDigest),
    /// Another existing prim (for example a descendant or a foreign prim).
    Prim(String),
    /// A target that did not resolve. Only foreign sources and partial indexes may hold one;
    /// an owned dangling link is an error.
    Unresolved(String),
}

/// One ordered relationship target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// Prim that carries the relationship.
    pub source_path: String,
    /// The element that is, or contains, the source prim.
    pub owner: Option<ElementId>,
    /// Whether every contributing site of the source prim is owned.
    pub source_owned: bool,
    /// Full relationship name.
    pub name: String,
    /// Position in the composed ordered target list.
    pub ordinal: usize,
    /// Resolved target.
    pub target: LinkTarget,
    /// The indexed element that is, or contains, the target prim. A link to a descendant such as
    /// `<element>/Geometry` is a dependency of that element, so it is indexed under it.
    pub target_owner: Option<ElementId>,
}

/// Plain, comparable description of what an index covers. Carries no authority by itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexToken {
    /// Complete pinned identity of the snapshot.
    pub identity: Identity,
    /// Coverage of the snapshot.
    pub completeness: Completeness,
    /// Number of prims indexed.
    pub prim_count: usize,
    /// Commitment to identities, paths, kinds, lifecycle, provenance layer/path/ownership,
    /// definitions and links. Labels and layer byte digests are excluded, so a rename leaves it
    /// unchanged; full content equality is [`PartialEq`].
    pub structure_digest: ContentDigest,
}

/// Reconstructible lookup and relationship index over one pinned context.
#[derive(Debug, Clone)]
pub struct ElementIndex {
    context: Context,
    completeness: Completeness,
    prim_count: usize,
    elements: BTreeMap<ElementId, ElementEntry>,
    definitions: BTreeMap<ContentDigest, DefinitionEntry>,
    links: Vec<Link>,
    incoming: BTreeMap<LinkTarget, Vec<usize>>,
    digest: ContentDigest,
}
// Content equality ignores the context incarnation, so a reopened identical state compares equal.
impl PartialEq for ElementIndex {
    fn eq(&self, other: &Self) -> bool {
        self.completeness == other.completeness
            && self.prim_count == other.prim_count
            && self.digest == other.digest
            && self.elements == other.elements
            && self.definitions == other.definitions
            && self.links == other.links
    }
}
impl Eq for ElementIndex {}

impl ElementIndex {
    /// The context this index was derived from. It authorizes nothing unless still live.
    pub fn context(&self) -> &Context {
        &self.context
    }
    /// Coverage of the pinned snapshot.
    pub fn completeness(&self) -> Completeness {
        self.completeness
    }
    /// Identity, coverage and digest of this index.
    pub fn token(&self) -> IndexToken {
        IndexToken {
            identity: self.context.identity.clone(),
            completeness: self.completeness,
            prim_count: self.prim_count,
            structure_digest: self.digest,
        }
    }
    /// Commitment excluding display labels and layer byte digests: a rename leaves it unchanged.
    pub fn structure_digest(&self) -> ContentDigest {
        self.digest
    }
    /// Number of indexed elements.
    pub fn len(&self) -> usize {
        self.elements.len()
    }
    /// True when no element is indexed.
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }
    /// All indexed elements ordered by full UUID.
    pub fn elements(&self) -> impl Iterator<Item = &ElementEntry> {
        self.elements.values()
    }
    fn complete(&self) -> Result<(), Diagnostic> {
        match self.completeness {
            Completeness::Complete => Ok(()),
            Completeness::Partial => Err(unavailable("element index is partial")),
        }
    }
    /// Looks up an element. A missing element is `NOT_FOUND` only in a complete index; a partial
    /// index cannot prove absence and answers `DEPENDENCY_UNAVAILABLE`.
    pub fn element(&self, id: &ElementId) -> Result<&ElementEntry, Diagnostic> {
        match self.elements.get(id) {
            Some(entry) => Ok(entry),
            None => {
                self.complete()?;
                Err(fail(
                    ErrorCode::NotFound,
                    "element is not in the pinned view",
                ))
            }
        }
    }
    /// Looks up by canonical UUID text; short or malformed text is `INVALID_ID`.
    pub fn lookup(&self, id: &str) -> Result<&ElementEntry, Diagnostic> {
        self.element(&ElementId::parse(id)?)
    }
    /// Looks up by canonical element path; short or malformed paths are `INVALID_ID`.
    pub fn lookup_path(&self, path: &str) -> Result<&ElementEntry, Diagnostic> {
        let entry = self.element(&ElementId::from_path(path)?)?;
        if entry.path != path {
            return Err(fail(ErrorCode::NotFound, "element path does not resolve"));
        }
        Ok(entry)
    }
    /// Looks up an immutable definition by full digest.
    pub fn definition(&self, digest: &ContentDigest) -> Result<&DefinitionEntry, Diagnostic> {
        match self.definitions.get(digest) {
            Some(entry) => Ok(entry),
            None => {
                self.complete()?;
                Err(fail(
                    ErrorCode::NotFound,
                    "definition is not in the pinned view",
                ))
            }
        }
    }
    /// Relationship targets carried by the element or its descendants, in canonical order.
    pub fn outgoing(&self, id: &ElementId) -> Result<Vec<&Link>, Diagnostic> {
        self.complete()?;
        self.element(id)?;
        Ok(self
            .links
            .iter()
            .filter(|l| l.owner.as_ref() == Some(id))
            .collect())
    }
    fn incoming_to(&self, target: &LinkTarget) -> Vec<&Link> {
        self.incoming
            .get(target)
            .map(|v| v.iter().filter_map(|i| self.links.get(*i)).collect())
            .unwrap_or_default()
    }
    /// Every relationship entry anywhere in the pinned view that targets the element or any of
    /// its descendant prims. Incomplete evidence is `DEPENDENCY_UNAVAILABLE`, never "no links".
    pub fn incoming(&self, id: &ElementId) -> Result<Vec<&Link>, Diagnostic> {
        self.complete()?;
        self.element(id)?;
        Ok(self.incoming_to(&LinkTarget::Element(*id)))
    }
    /// Every relationship entry that targets the immutable definition.
    pub fn incoming_definition(&self, digest: &ContentDigest) -> Result<Vec<&Link>, Diagnostic> {
        self.complete()?;
        self.definition(digest)?;
        Ok(self.incoming_to(&LinkTarget::Definition(*digest)))
    }
    /// Incoming links from outside the element itself (its own prim and descendants excluded):
    /// the dependencies that forbid a delete.
    pub fn delete_blockers(&self, id: &ElementId) -> Result<Vec<&Link>, Diagnostic> {
        Ok(self
            .incoming(id)?
            .into_iter()
            .filter(|l| l.owner.as_ref() != Some(id))
            .collect())
    }
    /// Other elements whose relationships target the element.
    pub fn dependents(&self, id: &ElementId) -> Result<BTreeSet<ElementId>, Diagnostic> {
        Ok(self
            .delete_blockers(id)?
            .into_iter()
            .filter_map(|l| l.owner)
            .collect())
    }
    /// Links whose element target is deactivated: valid, but reported (`DV-REL-012`).
    pub fn deactivated_targets(&self) -> Result<Vec<&Link>, Diagnostic> {
        self.complete()?;
        Ok(self
            .links
            .iter()
            .filter(|l| {
                l.target_owner
                    .and_then(|t| self.elements.get(&t))
                    .is_some_and(|e| e.lifecycle == Lifecycle::Deactivated)
            })
            .collect())
    }
    /// Links preserved from foreign sources (or a partial view) that did not resolve.
    pub fn unresolved(&self) -> Vec<&Link> {
        self.links
            .iter()
            .filter(|l| matches!(l.target, LinkTarget::Unresolved(_)))
            .collect()
    }
    /// Plans the identity and link remapping of a single-element copy. It computes data only:
    /// the source is untouched and no element is created. A foreign source, a reused or equal
    /// ID, or any link outside the supported copy set is refused rather than copied incompletely.
    pub fn copy_plan(&self, source: &ElementId, fresh: &ElementId) -> Result<CopyPlan, Diagnostic> {
        self.complete()?;
        let entry = self.element(source)?;
        if entry.ownership != Ownership::Owned {
            return Err(fail(
                ErrorCode::ForeignReadOnly,
                "a foreign source cannot be copied",
            ));
        }
        if fresh == source || self.elements.contains_key(fresh) {
            return Err(fail(
                ErrorCode::DuplicateId,
                "copy identity is already in use",
            ));
        }
        let mut links = Vec::new();
        for link in self
            .links
            .iter()
            .filter(|l| l.owner.as_ref() == Some(source))
        {
            // A duplicate is a loose independent part. P11 instance creation authors its
            // canonical definition link explicitly; an ordinary duplicate drops that link.
            if matches!(entry.kind.as_deref(), Some("plate" | "beam" | "column"))
                && link.source_path == entry.path
                && matches!(
                    link.name.as_str(),
                    "bim:partOf" | "bim:connects" | "bim:instanceOf"
                )
            {
                continue;
            }
            let target = match &link.target {
                // Links carried by a descendant prim cannot be represented by the plan yet.
                _ if link.source_path != entry.path => {
                    return Err(fail(
                        ErrorCode::DependencyConflict,
                        "descendant relationship is outside the supported copy set",
                    )
                    .with_rule("DV-REL-011"));
                }
                // Self links are internal state of the element and follow the copy.
                LinkTarget::Element(t) if t == source => LinkTarget::Element(*fresh),
                // Immutable definitions stay shared.
                LinkTarget::Definition(d) => LinkTarget::Definition(*d),
                // A level assignment (D2) stays with the copy: the copy is on the same level.
                LinkTarget::Element(t) if link.name == "bim:level" => LinkTarget::Element(*t),
                _ => {
                    return Err(fail(
                        ErrorCode::DependencyConflict,
                        "relationship is outside the supported copy set",
                    )
                    .with_rule("DV-REL-011"));
                }
            };
            links.push(PlannedLink {
                name: link.name.clone(),
                ordinal: link.ordinal,
                target,
            });
        }
        Ok(CopyPlan {
            source: *source,
            id: *fresh,
            path: fresh.path(
                category_of_path(&entry.path)
                    .ok_or_else(|| invalid_id("invalid element category"))?,
            ),
            links,
        })
    }
}

/// One relationship entry of a planned copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedLink {
    /// Full relationship name.
    pub name: String,
    /// Position in the ordered target list.
    pub ordinal: usize,
    /// Remapped target: the fresh ID for self links, the same digest for definitions.
    pub target: LinkTarget,
}

/// Result of [`ElementIndex::copy_plan`]. Applying it, naming and engine geometry belong to the
/// copy command (P05); this is only the identity and link remap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyPlan {
    /// Unchanged source element.
    pub source: ElementId,
    /// Fresh identity of the copy.
    pub id: ElementId,
    /// Canonical path of the copy.
    pub path: String,
    /// Relationship entries of the copy.
    pub links: Vec<PlannedLink>,
}

/// Incremental builder over delivered pages. It rejects mixed contexts, gaps, replays, caps and
/// failures, and re-validates every item rather than trusting page content.
pub struct IndexBuilder {
    context: Context,
    budgets: Budgets,
    expected: usize,
    prims: Vec<Prim>,
    paths: BTreeSet<String>,
    links: usize,
    bytes: usize,
    scanned: usize,
    complete: bool,
    failed: bool,
}
impl IndexBuilder {
    /// Starts an index for the snapshot described by an open result.
    pub fn new(opened: &Opened) -> Result<Self, Diagnostic> {
        Self::start(opened.context.clone(), opened.budgets, opened.prim_count)
    }
    fn start(context: Context, budgets: Budgets, expected: usize) -> Result<Self, Diagnostic> {
        budgets.validate()?;
        if expected > budgets.prims {
            return Err(limit());
        }
        Ok(Self {
            context,
            budgets,
            expected,
            prims: Vec::new(),
            paths: BTreeSet::new(),
            links: 0,
            bytes: 0,
            scanned: 0,
            complete: false,
            failed: false,
        })
    }
    /// Admits the next page in order. Any error poisons the builder.
    pub fn push(&mut self, page: &Page, control: &Control) -> Result<(), Diagnostic> {
        let result = self.admit(page, control);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn admit(&mut self, page: &Page, control: &Control) -> Result<(), Diagnostic> {
        control.check()?;
        if self.failed {
            return Err(fail(
                ErrorCode::IntegrityError,
                "index builder already failed",
            ));
        }
        if !Arc::ptr_eq(&page.context.marker, &self.context.marker) {
            return Err(fail(
                ErrorCode::StaleWorkspace,
                "page belongs to a different context",
            ));
        }
        if self.complete {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "snapshot is already complete",
            ));
        }
        let delivered = self.prims.len();
        match page.offset.cmp(&delivered) {
            std::cmp::Ordering::Greater => {
                return Err(unavailable("an earlier snapshot page is missing"));
            }
            std::cmp::Ordering::Less => {
                return Err(fail(
                    ErrorCode::IntegrityError,
                    "snapshot page was replayed",
                ));
            }
            std::cmp::Ordering::Equal => (),
        }
        if page.items.len() > self.budgets.page_items {
            return Err(limit());
        }
        let end = delivered.checked_add(page.items.len()).ok_or_else(limit)?;
        if end > self.expected {
            return Err(fail(
                ErrorCode::IntegrityError,
                "pages exceed the declared prim count",
            ));
        }
        match &page.next {
            Some(next) => {
                if !Arc::ptr_eq(&next.context.marker, &self.context.marker)
                    || next.offset != end
                    || page.items.is_empty()
                {
                    return Err(fail(
                        ErrorCode::IntegrityError,
                        "inconsistent page continuation",
                    ));
                }
            }
            None => self.complete = true,
        }
        for prim in &page.items {
            control.check()?;
            let projected = self.project(prim)?;
            self.prims.push(projected);
        }
        Ok(())
    }
    /// Bounds and charges everything read from a caller-held prim before any of it is copied,
    /// and retains only the fields the index uses. Ignored property values are neither copied
    /// nor traversed, so no nesting depth or value size can reach the allocator or the stack.
    fn project(&mut self, prim: &Prim) -> Result<Prim, Diagnostic> {
        const USED: [&str; 7] = [
            "bim:id",
            "bim:name",
            "bim:elementKind",
            "bim:geometryMode",
            "bim:isDeactivated",
            "bim:contentDigest",
            "bim:definitionKind",
        ];
        let b = self.budgets;
        let long = |s: &str| s.len() > MAX_PATH_BYTES;
        if long(&prim.path) || prim.element_id.as_deref().is_some_and(long) {
            return Err(limit());
        }
        let id_bytes = prim.element_id.as_deref().map_or(0, str::len);
        charge(
            &mut self.bytes,
            prim.path.len().saturating_add(id_bytes).saturating_add(256),
            b.snapshot_bytes,
        )?;
        if !self.paths.insert(prim.path.clone()) {
            return Err(fail(
                ErrorCode::IntegrityError,
                "duplicate prim path in snapshot",
            ));
        }
        charge(&mut self.scanned, prim.properties.len(), b.values)?;
        let mut properties = Vec::new();
        for p in prim
            .properties
            .iter()
            .filter(|p| USED.contains(&p.name.as_str()))
        {
            if long(&p.name) || long(&p.type_name) {
                return Err(limit());
            }
            // The index reads text, boolean and the block/absent distinction; anything else becomes
            // an inert placeholder that no rule accepts.
            let value = match &p.value {
                None => None,
                Some(Value::Block) => Some(Value::Block),
                Some(Value::Bool(value)) => Some(Value::Bool(*value)),
                Some(Value::Text(s)) if !long(s) => Some(Value::Text(s.clone())),
                Some(_) => Some(Value::Dictionary(BTreeMap::new())),
            };
            let text_bytes = match &value {
                Some(Value::Text(s)) => s.len(),
                _ => 0,
            };
            charge(
                &mut self.bytes,
                p.name
                    .len()
                    .saturating_add(p.type_name.len())
                    .saturating_add(text_bytes)
                    .saturating_add(128),
                b.snapshot_bytes,
            )?;
            properties.push(Property {
                name: p.name.clone(),
                type_name: p.type_name.clone(),
                value,
                interpolation: p.interpolation.clone(),
            });
        }
        if prim.sources.len() > b.values || prim.relationships.len() > b.values {
            return Err(limit());
        }
        for source in &prim.sources {
            if long(&source.layer) || long(&source.path) {
                return Err(limit());
            }
            charge(
                &mut self.bytes,
                source
                    .layer
                    .len()
                    .saturating_add(source.path.len())
                    .saturating_add(128),
                b.snapshot_bytes,
            )?;
        }
        for relationship in &prim.relationships {
            if long(&relationship.name) || relationship.targets.len() > b.array_items {
                return Err(limit());
            }
            charge(
                &mut self.bytes,
                relationship.name.len().saturating_add(128),
                b.snapshot_bytes,
            )?;
            self.links = self
                .links
                .checked_add(relationship.targets.len())
                .filter(|n| *n <= MAX_LINKS)
                .ok_or_else(limit)?;
            for target in &relationship.targets {
                if long(target) {
                    return Err(limit());
                }
                charge(
                    &mut self.bytes,
                    target.len().saturating_add(32),
                    b.snapshot_bytes,
                )?;
            }
        }
        Ok(Prim {
            path: prim.path.clone(),
            element_id: prim.element_id.clone(),
            type_name: None,
            active: prim.active,
            kind: None,
            display_name: None,
            properties,
            relationships: prim.relationships.clone(),
            sources: prim.sources.clone(),
        })
    }
    /// Finishes a complete index; a missing, gapped or short delivery is `DEPENDENCY_UNAVAILABLE`.
    pub fn finish(self, control: &Control) -> Result<ElementIndex, Diagnostic> {
        if !self.is_complete() {
            return Err(self.failure());
        }
        self.resolve(Completeness::Complete, control)
    }
    /// Finishes with whatever in-order prefix arrived. The result is marked
    /// [`Completeness::Partial`] unless every page was in fact delivered.
    pub fn finish_available(self, control: &Control) -> Result<ElementIndex, Diagnostic> {
        if self.failed {
            return Err(self.failure());
        }
        let completeness = if self.is_complete() {
            Completeness::Complete
        } else {
            Completeness::Partial
        };
        self.resolve(completeness, control)
    }
    fn is_complete(&self) -> bool {
        !self.failed && self.complete && self.prims.len() == self.expected
    }
    fn failure(&self) -> Diagnostic {
        if self.failed {
            fail(ErrorCode::IntegrityError, "index builder already failed")
        } else {
            unavailable("snapshot pages are incomplete")
        }
    }
    fn resolve(
        mut self,
        completeness: Completeness,
        control: &Control,
    ) -> Result<ElementIndex, Diagnostic> {
        // Fixed phase order and path order: the reported failure never depends on delivery order.
        self.prims.sort_by(|a, b| a.path.cmp(&b.path));
        for prim in &self.prims {
            control.check()?;
            if prim.sources.is_empty() {
                return Err(unavailable("missing composition provenance"));
            }
        }
        let mut ids = Vec::with_capacity(self.prims.len());
        for prim in &self.prims {
            ids.push(identity(prim)?);
        }
        let mut seen = BTreeSet::new();
        for id in ids.iter().flatten() {
            if !seen.insert(*id) {
                return Err(fail(
                    ErrorCode::DuplicateId,
                    "two elements carry the same identity",
                )
                .with_rule("DV-BASE-002"));
            }
        }
        for (prim, id) in self.prims.iter().zip(&ids) {
            if id.is_some_and(|id| {
                ElementId::from_path(&prim.path).ok() != Some(id)
                    || property(prim, "bim:elementKind")
                        .and_then(text)
                        .and_then(ElementCategory::from_kind)
                        .is_some_and(|category| Some(category) != category_of_path(&prim.path))
            }) {
                return Err(invalid_id("element identity and canonical path differ"));
            }
        }
        let owned_prim = |p: &Prim| p.sources.iter().all(|s| s.owned);
        let foreign: BTreeSet<String> = self
            .prims
            .iter()
            .filter(|p| !owned_prim(p))
            .map(|p| p.path.clone())
            .collect();
        let mut elements = BTreeMap::new();
        let mut definitions = BTreeMap::new();
        for (prim, id) in self.prims.iter().zip(&ids) {
            control.check()?;
            let definition = DefinitionCategory::parse_path(&prim.path);
            let definition_leaf = [
                DefinitionCategory::Material,
                DefinitionCategory::Profile,
                DefinitionCategory::Fastener,
            ]
            .into_iter()
            .any(|category| {
                prim.path
                    .strip_prefix(category.prefix())
                    .is_some_and(|rest| !rest.contains('/'))
            });
            if definition.is_none()
                && (definition_leaf
                    || prim.path.starts_with("/World/Definitions/D_")
                    || (owned_prim(prim)
                        && property(prim, "bim:contentDigest").is_some()
                        && property(prim, "bim:definitionKind").is_some()))
            {
                return Err(invalid_id(
                    "definition path lacks its category and full SHA-256",
                ));
            }
            if let Some(id) = id {
                elements.insert(*id, element_entry(prim, *id, &foreign)?);
            } else if let Some((category, digest)) = definition {
                let digest_text = digest.to_string();
                let rest = digest_text.as_str();
                if property(prim, "bim:definitionKind")
                    .and_then(text)
                    .is_some_and(|kind| DefinitionCategory::for_kind(kind) != category)
                {
                    return Err(invalid_id("definition category and kind differ"));
                }
                if let Some(p) = property(prim, "bim:contentDigest")
                    && text(p) != Some(rest)
                {
                    return Err(corrupt());
                }
                if definitions
                    .insert(
                        digest,
                        DefinitionEntry {
                            digest,
                            path: prim.path.clone(),
                            sources: prim.sources.clone(),
                        },
                    )
                    .is_some()
                {
                    return Err(corrupt());
                }
            }
        }
        let lookup = Lookup {
            elements: &elements,
            definitions: &definitions,
            paths: &self.paths,
        };
        let mut links = Vec::new();
        for prim in &self.prims {
            let owned = owned_prim(prim);
            let owner = owner_of(&prim.path);
            for relationship in &prim.relationships {
                for (ordinal, raw) in relationship.targets.iter().enumerate() {
                    control.check()?;
                    let target = match lookup.classify(raw) {
                        Ok(target) => target,
                        Err(_) if !owned || completeness == Completeness::Partial => {
                            LinkTarget::Unresolved(raw.clone())
                        }
                        Err(error) => return Err(error),
                    };
                    let target_owner = match &target {
                        LinkTarget::Element(id) => Some(*id),
                        LinkTarget::Prim(path) => {
                            owner_of(path).filter(|id| elements.contains_key(id))
                        }
                        LinkTarget::Definition(_) | LinkTarget::Unresolved(_) => None,
                    };
                    links.push(Link {
                        source_path: prim.path.clone(),
                        owner,
                        source_owned: owned,
                        name: relationship.name.clone(),
                        ordinal,
                        target,
                        target_owner,
                    });
                }
            }
        }
        links.sort_by(|a, b| {
            (&a.source_path, &a.name, a.ordinal).cmp(&(&b.source_path, &b.name, b.ordinal))
        });
        let mut incoming: BTreeMap<LinkTarget, Vec<usize>> = BTreeMap::new();
        for (position, link) in links.iter().enumerate() {
            // A link to a descendant prim is a dependency of the element that contains it.
            let key = link
                .target_owner
                .map_or_else(|| link.target.clone(), LinkTarget::Element);
            incoming.entry(key).or_default().push(position);
        }
        let digest = structure_digest(
            completeness,
            self.prims.len(),
            &elements,
            &definitions,
            &links,
        )?;
        control.check()?;
        Ok(ElementIndex {
            context: self.context,
            completeness,
            prim_count: self.prims.len(),
            elements,
            definitions,
            links,
            incoming,
            digest,
        })
    }
}

fn property<'a>(prim: &'a Prim, name: &str) -> Option<&'a Property> {
    prim.properties.iter().find(|p| p.name == name)
}
fn text(property: &Property) -> Option<&str> {
    match &property.value {
        Some(Value::Text(s)) => Some(s),
        _ => None,
    }
}

/// Re-derives the element identity from every place it is stored; all must agree.
fn identity(prim: &Prim) -> Result<Option<ElementId>, Diagnostic> {
    let bim = property(prim, "bim:id");
    match (&prim.element_id, bim) {
        (None, None) => {
            let is_element_leaf = prim.path.rsplit_once('/').is_some_and(|(parent, leaf)| {
                ElementCategory::ALL.into_iter().any(|category| {
                    let prefix = category.prefix();
                    let (folder, code) = prefix.rsplit_once('/').unwrap_or(("", ""));
                    parent == folder && leaf.starts_with(code)
                })
            });
            if is_element_leaf {
                Err(invalid_id("element path lacks an authored identity"))
            } else {
                Ok(None)
            }
        }
        (Some(_), None) | (None, Some(_)) => {
            Err(invalid_id("snapshot identity and bim:id property disagree"))
        }
        (Some(id), Some(property)) => {
            if !matches!(property.type_name.as_str(), "string" | "token")
                || text(property) != Some(id.as_str())
            {
                return Err(invalid_id("snapshot identity and bim:id value disagree"));
            }
            ElementId::parse(id).map(Some)
        }
    }
}

fn label_ok(label: &str) -> bool {
    // Same character rule as the authoring validator (DV-BASE-003).
    !label.is_empty() && label.len() <= MAX_LABEL_BYTES && !label.chars().any(label_char_forbidden)
}

fn element_entry(
    prim: &Prim,
    id: ElementId,
    foreign: &BTreeSet<String>,
) -> Result<ElementEntry, Diagnostic> {
    let owned = prim.sources.iter().all(|s| s.owned);
    let label = match property(prim, "bim:name") {
        None => None,
        Some(p) => match text(p) {
            Some(s) if !owned || label_ok(s) => Some(s.to_owned()),
            // A foreign label is inert display data; an owned one must satisfy DV-BASE-003.
            _ if !owned => None,
            _ => {
                return Err(fail(ErrorCode::InvalidArgument, "invalid element label")
                    .with_rule("DV-BASE-003"));
            }
        },
    };
    let lifecycle = if crate::is_deactivated(prim)? {
        Lifecycle::Deactivated
    } else {
        Lifecycle::Active
    };
    Ok(ElementEntry {
        id,
        path: prim.path.clone(),
        kind: property(prim, "bim:elementKind")
            .and_then(text)
            .map(str::to_owned),
        label,
        lifecycle,
        usd_active: prim.active,
        ownership: if owned && !tainted(foreign, &prim.path, shield(prim).as_deref()) {
            Ownership::Owned
        } else {
            Ownership::Foreign
        },
        sources: prim.sources.clone(),
    })
}

/// A foreign ancestor or descendant also makes an owned prim read-only (LEW-01).
///
/// `shield` is the one exception: the arc holder of an R01 reference wrapper and everything below
/// it are foreign by design, and the wrapper above them stays owned (it may be moved or
/// relabelled without touching them). The holder's own shape is validated separately.
fn tainted(foreign: &BTreeSet<String>, path: &str, shield: Option<&str>) -> bool {
    let hidden = |p: &str| {
        shield.is_some_and(|s| p == s || p.strip_prefix(s).is_some_and(|r| r.starts_with('/')))
    };
    if foreign.contains(path) && !hidden(path) {
        return true;
    }
    if path
        .match_indices('/')
        .filter_map(|(i, _)| path.get(..i))
        .any(|ancestor| !ancestor.is_empty() && foreign.contains(ancestor) && !hidden(ancestor))
    {
        return true;
    }
    let prefix = format!("{path}/");
    foreign
        .range(prefix.clone()..)
        .take_while(|p| p.starts_with(&prefix))
        .any(|p| !hidden(p))
}

/// The path below which an R01 reference wrapper composes foreign content, if `prim` is one: the
/// element kind and the geometry mode must both be `reference`. The wrapper's exact shape is
/// validated by U02 before any write; a prim with only one of the two gets no exception, so its
/// foreign descendant makes it foreign and it can never be granted an owned write.
fn shield(prim: &Prim) -> Option<String> {
    let is = |name: &str| property(prim, name).and_then(text) == Some("reference");
    (is("bim:elementKind") && is("bim:geometryMode"))
        .then(|| format!("{}/Correction/Source", prim.path))
}

fn category_of_path(path: &str) -> Option<ElementCategory> {
    ElementCategory::ALL
        .into_iter()
        .find(|category| path.starts_with(category.prefix()))
}
fn owner_of(path: &str) -> Option<ElementId> {
    let category = category_of_path(path)?;
    let rest = path.strip_prefix(category.prefix())?;
    let leaf = rest.split('/').next()?;
    ElementId::from_path(&format!("{}{leaf}", category.prefix())).ok()
}

struct Lookup<'a> {
    elements: &'a BTreeMap<ElementId, ElementEntry>,
    definitions: &'a BTreeMap<ContentDigest, DefinitionEntry>,
    paths: &'a BTreeSet<String>,
}
impl Lookup<'_> {
    fn classify(&self, target: &str) -> Result<LinkTarget, Diagnostic> {
        if target.len() > MAX_PATH_BYTES {
            return Err(limit());
        }
        if target.contains(['.', '[', '{', '<', ':']) {
            return Err(fail(
                ErrorCode::UnsupportedProperty,
                "relationship target is not a plain prim path",
            ));
        }
        let dangling = || {
            fail(
                ErrorCode::DanglingRelationship,
                "relationship target does not resolve",
            )
            .with_rule("DV-REL-001")
        };
        if let Some(category) = category_of_path(target)
            && !target
                .strip_prefix(category.prefix())
                .is_some_and(|rest| rest.contains('/'))
        {
            let id = ElementId::from_path(target)?;
            return if self
                .elements
                .get(&id)
                .is_some_and(|entry| entry.path == target)
            {
                Ok(LinkTarget::Element(id))
            } else {
                Err(dangling())
            };
        }
        if let Some((_, digest)) = DefinitionCategory::parse_path(target) {
            return if self
                .definitions
                .get(&digest)
                .is_some_and(|entry| entry.path == target)
            {
                Ok(LinkTarget::Definition(digest))
            } else {
                Err(dangling())
            };
        }
        if [
            DefinitionCategory::Material,
            DefinitionCategory::Profile,
            DefinitionCategory::Fastener,
        ]
        .into_iter()
        .any(|category| {
            target
                .strip_prefix(category.prefix())
                .is_some_and(|rest| !rest.contains('/'))
        }) {
            return Err(invalid_id("definition target lacks a full SHA-256 digest"));
        }
        if self.paths.contains(target) {
            Ok(LinkTarget::Prim(target.to_owned()))
        } else {
            Err(dangling())
        }
    }
}

struct Commit(Sha256);
impl Commit {
    fn put(&mut self, bytes: &[u8]) -> Result<(), Diagnostic> {
        self.0
            .update(&(bytes.len() as u64).to_le_bytes())
            .map_err(|_| limit())?;
        self.0.update(bytes).map_err(|_| limit())
    }
    fn number(&mut self, n: usize) -> Result<(), Diagnostic> {
        self.put(&(n as u64).to_le_bytes())
    }
    fn sources(&mut self, sources: &[Provenance]) -> Result<(), Diagnostic> {
        self.number(sources.len())?;
        for s in sources {
            self.put(s.layer.as_bytes())?;
            self.put(s.path.as_bytes())?;
            self.put(&[u8::from(s.owned)])?;
        }
        Ok(())
    }
}

fn structure_digest(
    completeness: Completeness,
    prims: usize,
    elements: &BTreeMap<ElementId, ElementEntry>,
    definitions: &BTreeMap<ContentDigest, DefinitionEntry>,
    links: &[Link],
) -> Result<ContentDigest, Diagnostic> {
    let mut c = Commit(Sha256::new());
    c.put(b"BeamCraft/U03/index/v1")?;
    c.put(&[u8::from(completeness == Completeness::Complete)])?;
    c.number(prims)?;
    c.number(elements.len())?;
    for e in elements.values() {
        c.put(&e.id.as_u128().to_be_bytes())?;
        c.put(e.path.as_bytes())?;
        c.put(e.kind.as_deref().unwrap_or("").as_bytes())?;
        c.put(&[
            u8::from(e.kind.is_some()),
            u8::from(e.lifecycle == Lifecycle::Deactivated),
            u8::from(e.usd_active),
            u8::from(e.ownership == Ownership::Owned),
        ])?;
        c.sources(&e.sources)?;
    }
    c.number(definitions.len())?;
    for d in definitions.values() {
        c.put(d.digest.as_bytes())?;
        c.put(d.path.as_bytes())?;
        c.sources(&d.sources)?;
    }
    c.number(links.len())?;
    for l in links {
        c.put(l.source_path.as_bytes())?;
        c.put(&[u8::from(l.source_owned)])?;
        c.put(l.name.as_bytes())?;
        c.number(l.ordinal)?;
        match &l.target {
            LinkTarget::Element(id) => {
                c.put(b"element")?;
                c.put(&id.as_u128().to_be_bytes())?;
            }
            LinkTarget::Definition(d) => {
                c.put(b"definition")?;
                c.put(d.as_bytes())?;
            }
            LinkTarget::Prim(p) => {
                c.put(b"prim")?;
                c.put(p.as_bytes())?;
            }
            LinkTarget::Unresolved(p) => {
                c.put(b"unresolved")?;
                c.put(p.as_bytes())?;
            }
        }
    }
    Ok(c.0.finalize())
}

impl Reader {
    /// Builds the complete index of the pinned snapshot by paging the frozen result through
    /// [`IndexBuilder`]. It never reopens or recomposes the stage.
    pub fn element_index(
        &self,
        context: &Context,
        control: &Control,
    ) -> Result<ElementIndex, Diagnostic> {
        let stage = self.pinned(context)?;
        let mut builder = IndexBuilder::start(context.clone(), stage.budgets, stage.prims.len())?;
        let mut cursor: Option<Cursor> = None;
        loop {
            let page = self.read(context, cursor.as_ref(), control)?;
            builder.push(&page, control)?;
            match page.next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        builder.finish(control)
    }
    /// Proves the index is complete, derived from this reader's live context and equal to a
    /// fresh rebuild. A stale context is `STALE_WORKSPACE`, a partial index
    /// `DEPENDENCY_UNAVAILABLE` and a forged or tampered one `INTEGRITY_ERROR`.
    pub fn verify_index(&self, index: &ElementIndex, control: &Control) -> Result<(), Diagnostic> {
        self.pinned(&index.context)?;
        index.complete()?;
        if self.element_index(&index.context, control)? != *index {
            return Err(fail(
                ErrorCode::IntegrityError,
                "index does not match the pinned snapshot",
            ));
        }
        Ok(())
    }
}

/// Reads and writes colliding on one element: a lifecycle write (delete, deactivate) on an
/// element collides with any read of it or of an element that has a relationship to it.
pub(crate) fn dependent_collision(
    index: &ElementIndex,
    writes: &BTreeSet<Footprint>,
    reads: &BTreeSet<Footprint>,
) -> Result<(), Diagnostic> {
    index.complete()?;
    let mut read_elements = BTreeSet::new();
    for read in reads {
        if let Some(id) = footprint_element(&read.identity)? {
            read_elements.insert(id);
        }
    }
    for write in writes {
        let element = footprint_element(&write.identity)?;
        let Some(id) = element.filter(|_| write.group == Group::Lifecycle) else {
            continue;
        };
        // The written ID itself always collides with a read of it, even when it is absent (a
        // read of a fresh ID's absence conflicts with creating it). Dependents exist only for an
        // indexed element, where complete evidence is required.
        let mut affected = BTreeSet::from([id]);
        if index.elements.contains_key(&id) {
            affected.extend(index.dependents(&id)?);
        }
        if affected.iter().any(|e| read_elements.contains(e)) {
            return Err(fail(
                ErrorCode::DependencyConflict,
                "lifecycle write collides with a dependent read",
            ));
        }
    }
    Ok(())
}
