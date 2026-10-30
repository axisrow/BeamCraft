// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! C03.3: canonical engine-command footprints pinned as opaque revision evidence.
//!
//! The producer receives the actual admitted command's footprint, never one inferred from a
//! USD difference. The host pins its bytes in the immutable manifest. Decoding checks structure
//! and the creating operation identity; it grants no permissions or missing command provenance.

use crate::plate::{PlateError, integrity};
use beamcraft_engine::footprint::{Entry, Footprint, Group, Target};
use beamcraft_host::revision::{
    EvidencePin, MAX_MANIFEST_BYTES, MAX_OPERATION_EVIDENCE_BYTES, Manifest, Revision,
    RevisionInfo, RevisionStore, Uuid7,
};
use beamcraft_model::changeset::{
    Access, Dependency, DependencyGroup, DependencyTarget, MAX_OPERATIONS, Operation,
};
use beamcraft_model::command::CommandError;
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId};
use beamcraft_model::limits::MAX_FOOTPRINT_ENTRIES;
use std::fmt::Write as _;

const MAGIC: &str = "beamcraft-command-footprint 1";

/// A command/publication identity, request fingerprint and actual semantic read/write set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedFootprint {
    /// Full operation identity; it must match the revision manifest.
    pub op_id: OpId,
    /// A typed command's argument digest or an aggregate publication request's immutable
    /// source/payload digest. This is not the unimplemented RFC 8785 envelope.
    pub fingerprint: ContentDigest,
    /// Actual engine-derived semantic dependencies.
    pub footprint: Footprint,
}

fn invalid() -> CommandError {
    CommandError::new(
        ErrorCode::InvalidArgument,
        "operation_evidence",
        "operation footprint evidence is not canonical",
    )
}

fn group(text: &str) -> Result<Group, CommandError> {
    Ok(match text {
        "label" => Group::Label,
        "shape_inputs" => Group::ShapeInputs,
        "placement" => Group::Placement,
        "material_ref" => Group::MaterialRef,
        "relationship" => Group::Relationship,
        "mark" => Group::Mark,
        "attributes" => Group::Attributes,
        "lifecycle" => Group::Lifecycle,
        "derived" => Group::Derived,
        _ => return Err(invalid()),
    })
}

fn field<'a>(
    lines: &mut impl Iterator<Item = &'a str>,
    name: &str,
) -> Result<&'a str, CommandError> {
    lines
        .next()
        .and_then(|line| line.strip_prefix(name))
        .and_then(|tail| tail.strip_prefix(' '))
        .ok_or_else(invalid)
}

fn entries<'a>(
    lines: &mut impl Iterator<Item = &'a str>,
    count: usize,
) -> Result<Vec<Entry>, CommandError> {
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let mut fields = lines.next().ok_or_else(invalid)?.split(' ');
        let (Some(kind), Some(id), Some(name), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err(invalid());
        };
        let target = match kind {
            "element" => Target::Element(ElementId::parse(id).map_err(|_| invalid())?),
            "definition" => Target::Definition(id.parse().map_err(|_| invalid())?),
            "schema" if id == beamcraft_layer_engine::authored::ATTR_SCHEMA_IDENTITY => {
                Target::AttrSchema
            }
            _ => return Err(invalid()),
        };
        let entry = Entry {
            target,
            group: group(name)?,
        };
        if entries.last().is_some_and(|previous| *previous >= entry) {
            return Err(invalid());
        }
        entries.push(entry);
    }
    Ok(entries)
}

impl CapturedFootprint {
    /// Exact canonical bytes. Only nonempty, capped engine-command footprints are admitted.
    pub fn encode(&self) -> Result<Vec<u8>, CommandError> {
        let count = self
            .footprint
            .reads()
            .len()
            .saturating_add(self.footprint.writes().len());
        if count == 0 || count > MAX_FOOTPRINT_ENTRIES {
            return Err(invalid());
        }
        let mut out = format!("{MAGIC}\nop {}\nrequest {}\n", self.op_id, self.fingerprint);
        // Writing into an owned String cannot fail.
        for (name, entries) in [
            ("reads", self.footprint.reads()),
            ("writes", self.footprint.writes()),
        ] {
            let _ = writeln!(out, "{name} {}", entries.len());
            for entry in entries {
                let (kind, identity) = match entry.target {
                    Target::Element(id) => ("element", id.to_string()),
                    Target::Definition(digest) => ("definition", digest.to_string()),
                    Target::AttrSchema => (
                        "schema",
                        beamcraft_layer_engine::authored::ATTR_SCHEMA_IDENTITY.to_string(),
                    ),
                };
                let _ = writeln!(out, "{kind} {identity} {}", entry.group.as_str());
            }
        }
        if out.len() as u64 > MAX_OPERATION_EVIDENCE_BYTES {
            return Err(invalid());
        }
        Ok(out.into_bytes())
    }

    /// Bounded, strict parser: closed tags, full identities, sorted unique entries, exact counts
    /// and a byte-identical canonical re-encoding. Missing evidence is handled separately.
    pub fn decode(bytes: &[u8]) -> Result<Self, CommandError> {
        if bytes.len() as u64 > MAX_OPERATION_EVIDENCE_BYTES {
            return Err(invalid());
        }
        let text = std::str::from_utf8(bytes).map_err(|_| invalid())?;
        let mut lines = text.strip_suffix('\n').ok_or_else(invalid)?.split('\n');
        if lines.next() != Some(MAGIC) {
            return Err(invalid());
        }
        let op_id = OpId::parse(field(&mut lines, "op")?).map_err(|_| invalid())?;
        let fingerprint = field(&mut lines, "request")?
            .parse()
            .map_err(|_| invalid())?;
        let count = |value: &str| {
            value
                .parse::<usize>()
                .ok()
                .filter(|n| *n <= MAX_FOOTPRINT_ENTRIES)
                .ok_or_else(invalid)
        };
        let reads_count = count(field(&mut lines, "reads")?)?;
        let reads = entries(&mut lines, reads_count)?;
        let writes_count = count(field(&mut lines, "writes")?)?;
        if reads_count.saturating_add(writes_count) > MAX_FOOTPRINT_ENTRIES {
            return Err(invalid());
        }
        let writes = entries(&mut lines, writes_count)?;
        if lines.next().is_some() {
            return Err(invalid());
        }
        let captured = Self {
            op_id,
            fingerprint,
            footprint: Footprint::from_entries(&reads, &writes)?,
        };
        if captured.encode()? != bytes {
            return Err(invalid());
        }
        Ok(captured)
    }
}

/// Reads pinned evidence from a verified revision and checks that it names this operation.
/// Legacy absence is explicit; malformed evidence is an integrity error, never an empty set.
pub fn read(revision: &Revision) -> Result<Option<CapturedFootprint>, PlateError> {
    let Some(bytes) = revision.operation_evidence()? else {
        return Ok(None);
    };
    Ok(Some(decode_for(
        &bytes,
        revision.manifest().content().op(),
    )?))
}

/// Resolve a committed logical request from immutable history, including its exact manifest
/// and operation identity. Legacy missing evidence refuses rather than guessing from live state.
pub(crate) fn accepted(
    store: &RevisionStore,
    op: &Uuid7,
) -> Result<Option<(CapturedFootprint, Revision)>, PlateError> {
    match store.query_outcome(op)? {
        beamcraft_host::ledger::Outcome::Committed {
            sequence,
            manifest,
            revision,
            ..
        } => {
            let opened = store.open_at(sequence)?;
            let info = opened.info();
            if info.manifest_digest() != manifest
                || info.manifest().content().revision() != &revision
                || info.manifest().content().op() != op
            {
                return Err(integrity("ledger request and accepted revision disagree"));
            }
            let evidence = read(&opened)?.ok_or_else(|| {
                PlateError::new(
                    ErrorCode::DependencyUnavailable,
                    "accepted operation has no compatible request evidence",
                )
            })?;
            Ok(Some((evidence, opened)))
        }
        beamcraft_host::ledger::Outcome::Unknown => Err(PlateError::new(
            ErrorCode::OutcomeUnknown,
            "operation outcome requires recovery",
        )),
        _ => Ok(None),
    }
}

fn decode_for(bytes: &[u8], op: &Uuid7) -> Result<CapturedFootprint, PlateError> {
    let captured = if bytes.starts_with(b"beamcraft-workspace-rebase ") {
        let receipt = crate::workspace::rebase_receipt::RebaseReceipt::decode(bytes)
            .map_err(|_| integrity("rebase evidence is malformed"))?;
        CapturedFootprint {
            op_id: receipt.op_id,
            fingerprint: receipt.fingerprint()?,
            footprint: receipt.footprint,
        }
    } else {
        CapturedFootprint::decode(bytes)
            .map_err(|_| integrity("operation footprint evidence is malformed"))?
    };
    if captured.op_id.to_string() != op.as_str() {
        return Err(integrity(
            "operation footprint evidence belongs to another operation",
        ));
    }
    Ok(captured)
}

pub(crate) fn store(
    store: &RevisionStore,
    op: &Uuid7,
    fingerprint: ContentDigest,
    footprint: &Footprint,
) -> Result<EvidencePin, PlateError> {
    let captured = CapturedFootprint {
        op_id: OpId::parse(op.as_str()).map_err(|_| integrity("operation identity is invalid"))?,
        fingerprint,
        footprint: footprint.clone(),
    };
    let bytes = captured.encode()?;
    let (digest, _) = store
        .blobs()
        .publish(&bytes)
        .map_err(|e| PlateError::new(e.code(), "operation evidence blob could not be stored"))?;
    Ok(EvidencePin::new(digest, bytes.len() as u64)?)
}

pub(crate) struct SourceInterval {
    pub(crate) head: RevisionInfo,
    /// Union of the retained modelling dependencies: renounced work is excluded. This drives
    /// planning, materialization and publication evidence.
    pub(crate) footprint: Option<Footprint>,
    /// Union over the complete raw interval including renounced work. A rebase receipt
    /// describes its whole source interval with this, so it stays nonempty even when every
    /// command was renounced.
    pub(crate) raw_footprint: Option<Footprint>,
    pub(crate) commands: Vec<beamcraft_engine::rebase::Operation>,
}

/// Verify the complete contiguous source interval. Absence is conservative, not an empty proof;
/// later malformed records are still checked after an absent one. No publication happens here.
pub(crate) fn aggregate(
    store: &RevisionStore,
    after: u64,
    operations: &[Operation],
) -> Result<SourceInterval, PlateError> {
    if operations.is_empty() || operations.len() > MAX_OPERATIONS {
        return Err(PlateError::new(
            ErrorCode::LimitExceeded,
            "operation interval is empty or exceeds its cap",
        ));
    }
    let mut previous = after;
    let mut head: Option<RevisionInfo> = None;
    let mut combined = Footprint::default();
    let mut raw = Footprint::default();
    let mut complete = true;
    let mut raw_entries = 0usize;
    let mut commands = Vec::with_capacity(operations.len());
    let mut last_rebase: Option<(u64, crate::workspace::rebase_receipt::RevisionPin)> = None;
    for (position, operation) in operations.iter().enumerate() {
        let bytes = store
            .blobs()
            .read_capped(&operation.result, MAX_MANIFEST_BYTES)
            .map_err(|_| integrity("source operation manifest is unavailable"))?;
        let manifest = Manifest::parse(&bytes)?;
        let next = previous
            .checked_add(1)
            .ok_or_else(|| integrity("source sequence overflow"))?;
        if manifest.sequence() != next {
            return Err(integrity(
                "source operations are not a complete ordered interval",
            ));
        }
        let info = store.info_at(next)?;
        let content = info.manifest().content();
        if info.manifest_digest() != operation.result
            || content.op().as_str() != operation.op_id.to_string()
        {
            return Err(integrity(
                "source operation identity or result disagrees with history",
            ));
        }
        if let Some(prior) = head.as_ref() {
            let old = prior.manifest().content();
            if old.project() != content.project()
                || old.workspace() != content.workspace()
                || old.schema() != content.schema()
            {
                return Err(integrity(
                    "source operation context changed within one interval",
                ));
            }
        }
        match content.operation_evidence() {
            None => complete = false,
            Some(pin) => {
                let bytes = store.evidence_bytes(pin)?;
                if bytes.starts_with(b"beamcraft-workspace-rebase ") {
                    let receipt = crate::workspace::rebase_receipt::RebaseReceipt::decode(&bytes)
                        .map_err(|_| integrity("rebase evidence is malformed"))?;
                    let original = head
                        .as_ref()
                        .ok_or_else(|| integrity("rebase has no source operations"))?;
                    let expected =
                        crate::workspace::rebase_receipt::RevisionPin::from_info(original);
                    if !complete
                        || receipt.op_id != operation.op_id
                        || receipt.draft != expected
                        || receipt.operations.as_slice()
                            != operations.get(..position).unwrap_or_default()
                        || receipt.old_meta.synced_sequence != after
                        || receipt.old_meta.workspace_id != *content.workspace()
                        || receipt.project != *content.project()
                        || receipt.schema != content.schema()
                        || receipt.footprint != raw
                        || info.manifest().index().is_none()
                        || last_rebase.as_ref().is_some_and(|(boundary, accepted)| {
                            receipt.old_meta.rebase_sequence != *boundary
                                || receipt.old_meta.base_revision != accepted.revision
                                || receipt.old_meta.base_manifest != accepted.manifest
                                || receipt.old_base_sequence != accepted.sequence
                        })
                    {
                        return Err(integrity(
                            "rebase receipt disagrees with its verified source interval",
                        ));
                    }
                    last_rebase = Some((next, receipt.accepted));
                    // An administrative receipt retains the earlier commands; it is not a second
                    // copy of their modelling work and consumes no additional footprint budget.
                    previous = next;
                    head = Some(info);
                    continue;
                }
                if bytes.starts_with(b"beamcraft-workspace-discard ") {
                    let receipt = crate::workspace::resolve::DiscardReceipt::decode(&bytes)
                        .map_err(|_| integrity("discard receipt is malformed"))?;
                    let original = head
                        .as_ref()
                        .ok_or_else(|| integrity("discard has no source operations"))?;
                    if receipt.op_id != operation.op_id
                        || receipt.head
                            != crate::workspace::rebase_receipt::RevisionPin::from_info(original)
                        || receipt.workspace != *content.workspace()
                        || receipt.project != *content.project()
                        || receipt.schema != content.schema()
                    {
                        return Err(integrity(
                            "discard receipt disagrees with its verified source interval",
                        ));
                    }
                    let kept = commands
                        .iter()
                        .rposition(|command: &beamcraft_engine::rebase::Operation| {
                            command.id == receipt.discarded
                        })
                        .ok_or_else(|| {
                            integrity("discard receipt names no retained source operation")
                        })?;
                    commands.truncate(kept);
                    combined = commands
                        .iter()
                        .try_fold(Footprint::default(), |union, command| {
                            union.union(command.footprint.clone())
                        })?;
                    // An administrative resolution is not a second modelling command: the
                    // renounced operations leave the retained set and the receipt itself
                    // carries no footprint.
                    previous = next;
                    head = Some(info);
                    continue;
                }
                let captured = decode_for(&bytes, content.op())?;
                if info.manifest().index().is_none() {
                    complete = false;
                }
                raw_entries = raw_entries
                    .saturating_add(captured.footprint.reads().len())
                    .saturating_add(captured.footprint.writes().len());
                if raw_entries > MAX_FOOTPRINT_ENTRIES {
                    return Err(PlateError::new(
                        ErrorCode::LimitExceeded,
                        "aggregate operation footprint exceeds 4096 entries",
                    ));
                }
                commands.push(beamcraft_engine::rebase::Operation {
                    id: captured.op_id,
                    footprint: captured.footprint.clone(),
                });
                combined = combined.union(captured.footprint.clone())?;
                raw = raw.union(captured.footprint)?;
            }
        }
        previous = next;
        head = Some(info);
    }
    Ok(SourceInterval {
        head: head.ok_or_else(|| integrity("source interval has no head"))?,
        footprint: complete.then_some(combined),
        raw_footprint: complete.then_some(raw),
        commands,
    })
}

pub(crate) fn interval(
    store: &RevisionStore,
    after: u64,
    through: u64,
) -> Result<Vec<Operation>, PlateError> {
    let count = through
        .checked_sub(after)
        .filter(|n| *n > 0 && *n <= MAX_OPERATIONS as u64)
        .ok_or_else(|| {
            PlateError::new(
                ErrorCode::LimitExceeded,
                "source interval exceeds operation limits",
            )
        })?;
    let mut operations = Vec::with_capacity(usize::try_from(count).map_err(|_| invalid())?);
    for sequence in after.saturating_add(1)..=through {
        let info = store.info_at(sequence)?;
        operations.push(Operation {
            op_id: OpId::parse(info.manifest().content().op().as_str())
                .map_err(|_| integrity("source operation identity is invalid"))?,
            result: info.manifest_digest(),
        });
    }
    Ok(operations)
}

pub(crate) fn dependencies(footprint: &Footprint) -> Vec<Dependency> {
    let mut result = Vec::with_capacity(
        footprint
            .reads()
            .len()
            .saturating_add(footprint.writes().len()),
    );
    for (entries, access) in [
        (footprint.reads(), Access::Read),
        (footprint.writes(), Access::Write),
    ] {
        for entry in entries {
            let target = match entry.target {
                Target::Element(id) => DependencyTarget::Element(id),
                Target::Definition(digest) => DependencyTarget::Definition(digest),
                Target::AttrSchema => DependencyTarget::AttrSchema,
            };
            let group = match entry.group {
                Group::Label => DependencyGroup::Label,
                Group::ShapeInputs => DependencyGroup::ShapeInputs,
                Group::Placement => DependencyGroup::Placement,
                Group::MaterialRef => DependencyGroup::MaterialRef,
                Group::Relationship => DependencyGroup::Relationship,
                Group::Mark => DependencyGroup::Mark,
                Group::Attributes => DependencyGroup::Attributes,
                Group::Lifecycle => DependencyGroup::Lifecycle,
                Group::Derived => DependencyGroup::Derived,
            };
            result.push(Dependency {
                target,
                group,
                access,
            });
        }
    }
    result.sort();
    result
}
