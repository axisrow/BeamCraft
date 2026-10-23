// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! C03.3: canonical engine-command footprints pinned as opaque revision evidence.
//!
//! The producer receives the actual admitted command's footprint, never one inferred from a
//! USD difference. The host pins its bytes in the immutable manifest. Decoding checks structure
//! and the creating operation identity; it grants no permissions or missing command provenance.

use crate::plate::{PlateError, integrity};
use beamcraft_engine::footprint::{Entry, Footprint, Group, Target};
use beamcraft_host::revision::{MAX_OPERATION_EVIDENCE_BYTES, Revision};
use beamcraft_model::command::CommandError;
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId};
use beamcraft_model::limits::MAX_FOOTPRINT_ENTRIES;
use std::fmt::Write as _;

const MAGIC: &str = "beamcraft-command-footprint 1";

/// The creating engine command's exact identity, typed-argument fingerprint and read/write set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedFootprint {
    /// Full operation identity; it must match the revision manifest.
    pub op_id: OpId,
    /// Existing command-specific typed-argument digest, not the unimplemented RFC 8785 envelope.
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
    let captured = CapturedFootprint::decode(&bytes)
        .map_err(|_| integrity("operation footprint evidence is malformed"))?;
    if captured.op_id.to_string() != revision.manifest().content().op().as_str() {
        return Err(integrity(
            "operation footprint evidence belongs to another operation",
        ));
    }
    Ok(Some(captured))
}
