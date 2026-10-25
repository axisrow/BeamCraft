// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! C04: the durable record of unresolved semantic rebase conflicts.
//!
//! A conflicted rebase changes neither head: it writes this record and nothing else. While a
//! record stays active, publication and packaging refuse until the draft is explicitly resolved
//! (an edit or undo that no longer leaves the conflicting operation unsent). The record is
//! canonical bytes: the same inputs produce the same bytes on every retry and in every process.

use super::WorkspaceMeta;
use crate::plate::{PlateError, integrity};
use beamcraft_engine::{
    footprint::{Entry, Group, Target},
    rebase::{Conflict, ConflictKind, MAX_REBASE_CONFLICTS},
};
use beamcraft_host::revision::{RevisionStore, Uuid7};
use beamcraft_json::Json;
use beamcraft_model::{digest::ContentDigest, error::ErrorCode, id::ElementId};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

const FILE: &str = "conflicts.pending";
const BYTES_CAP: u64 = 65536;

fn bad() -> PlateError {
    integrity("recorded workspace conflicts are not usable")
}

/// The recorded conflicts of the latest conflicted rebase, bound to its accepted base pin and
/// the exact draft head it planned against. A moved base or draft head resolves the record:
/// every forward path then goes through a fresh rebase, which re-plans deterministically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingConflicts {
    /// The workspace base whose accepted interval conflicts.
    pub base_revision: Uuid7,
    /// Manifest digest of that base.
    pub base_manifest: ContentDigest,
    /// The draft head the conflicted plan saw.
    pub draft_head: Uuid7,
    /// Manifest digest of that draft head.
    pub draft_manifest: ContentDigest,
    /// Complete bounded report in planner order.
    pub conflicts: Vec<Conflict>,
}

impl PendingConflicts {
    fn encode(&self) -> Result<String, PlateError> {
        if self.conflicts.len() > MAX_REBASE_CONFLICTS {
            return Err(bad());
        }
        let mut out = format!(
            "beamcraft-conflicts 1\nbase_revision={}\nbase_manifest={}\ndraft_head={}\ndraft_manifest={}\n",
            self.base_revision.as_str(),
            self.base_manifest,
            self.draft_head.as_str(),
            self.draft_manifest
        );
        for conflict in &self.conflicts {
            out.push_str(&format!(
                "conflict={}|{}|{}:{}|{}|{}\n",
                conflict.draft,
                conflict.accepted,
                match conflict.entry.target {
                    Target::Element(_) => "element",
                    Target::Definition(_) => "definition",
                },
                match conflict.entry.target {
                    Target::Element(id) => id.to_string(),
                    Target::Definition(digest) => digest.to_string(),
                },
                conflict.entry.group.as_str(),
                conflict.kind.as_str()
            ));
        }
        Ok(out)
    }

    fn decode(text: &str) -> Result<Self, PlateError> {
        let mut lines = text.lines();
        if lines.next() != Some("beamcraft-conflicts 1") {
            return Err(bad());
        }
        let mut field = |key: &str| -> Result<&str, PlateError> {
            lines
                .next()
                .and_then(|l| l.strip_prefix(key))
                .and_then(|l| l.strip_prefix('='))
                .ok_or_else(bad)
        };
        let base_revision = Uuid7::parse(field("base_revision")?).map_err(|_| bad())?;
        let base_manifest = field("base_manifest")?.parse().map_err(|_| bad())?;
        let draft_head = Uuid7::parse(field("draft_head")?).map_err(|_| bad())?;
        let draft_manifest = field("draft_manifest")?.parse().map_err(|_| bad())?;
        let mut conflicts = Vec::new();
        for line in lines {
            let rest = line.strip_prefix("conflict=").ok_or_else(bad)?;
            let parts: Vec<&str> = rest.split('|').collect();
            let [draft, accepted, target_text, group, kind] = parts.as_slice() else {
                return Err(bad());
            };
            let draft = beamcraft_model::id::OpId::parse(draft).map_err(|_| bad())?;
            let accepted = beamcraft_model::id::OpId::parse(accepted).map_err(|_| bad())?;
            let target = match target_text.split_once(':') {
                Some(("element", id)) => Target::Element(ElementId::parse(id).map_err(|_| bad())?),
                Some(("definition", digest)) => {
                    Target::Definition(digest.parse().map_err(|_| bad())?)
                }
                _ => return Err(bad()),
            };
            let group = match *group {
                "label" => Group::Label,
                "shape_inputs" => Group::ShapeInputs,
                "placement" => Group::Placement,
                "material_ref" => Group::MaterialRef,
                "relationship" => Group::Relationship,
                "mark" => Group::Mark,
                "lifecycle" => Group::Lifecycle,
                "derived" => Group::Derived,
                _ => return Err(bad()),
            };
            let kind = match *kind {
                "read_write" => ConflictKind::ReadWrite,
                "write_write" => ConflictKind::WriteWrite,
                "lifecycle" => ConflictKind::Lifecycle,
                _ => return Err(bad()),
            };
            conflicts.push(Conflict {
                draft,
                accepted,
                entry: Entry { target, group },
                kind,
            });
        }
        let record = Self {
            base_revision,
            base_manifest,
            draft_head,
            draft_manifest,
            conflicts,
        };
        if record.encode()? != text {
            return Err(bad());
        }
        Ok(record)
    }
}

/// One bounded deterministic JSON diagnostic of a conflict, shared by every report surface.
pub fn conflict_json(conflict: &Conflict) -> Json {
    let (kind, target) = match conflict.entry.target {
        Target::Element(id) => ("element", id.to_string()),
        Target::Definition(digest) => ("definition", digest.to_string()),
    };
    Json::object([
        ("draft_op", Json::text(conflict.draft.to_string())),
        ("accepted_op", Json::text(conflict.accepted.to_string())),
        ("target_kind", Json::text(kind)),
        ("target", Json::text(target)),
        ("group", Json::text(conflict.entry.group.as_str())),
        ("kind", Json::text(conflict.kind.as_str())),
    ])
}

/// The typed refusal that gates the draft while conflicts stand; the named list travels in
/// the status and rejection diagnostics.
pub(crate) fn refusal() -> PlateError {
    PlateError::new(
        ErrorCode::DependencyConflict,
        "unresolved semantic conflicts against the accepted base block this draft; \
         inspect-workspace lists every conflict",
    )
}

fn read(dir: &Path) -> Result<Option<PendingConflicts>, PlateError> {
    let path = dir.join(FILE);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(metadata) if metadata.is_file() && metadata.len() <= BYTES_CAP => {}
        _ => return Err(bad()),
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|_| bad())?
        .take(BYTES_CAP.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| bad())?;
    if bytes.len() as u64 > BYTES_CAP {
        return Err(bad());
    }
    PendingConflicts::decode(std::str::from_utf8(&bytes).map_err(|_| bad())?).map(Some)
}

pub(crate) fn store(dir: &Path, record: &PendingConflicts) -> Result<(), PlateError> {
    let bytes = record.encode()?;
    if let Some(previous) = read(dir)?
        && previous.encode()? == bytes
    {
        return Ok(());
    }
    let temporary = dir.join("conflicts.pending.tmp");
    match fs::remove_file(&temporary) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(bad()),
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| bad())?;
    file.write_all(bytes.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|_| bad())?;
    fs::rename(&temporary, dir.join(FILE)).map_err(|_| bad())
}

fn clear(dir: &Path) -> Result<(), PlateError> {
    match fs::remove_file(dir.join(FILE)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(bad()),
    }
}

/// The still-active conflicts: the recorded base pin is the current base and the draft head is
/// still the exact revision the conflicted plan saw. Any move of either head resolves the
/// record, because every forward path then runs a fresh rebase that re-plans deterministically.
pub(crate) fn active(
    workspace: &RevisionStore,
    dir: &Path,
    meta: &WorkspaceMeta,
) -> Result<Vec<Conflict>, PlateError> {
    let Some(record) = read(dir)? else {
        return Ok(Vec::new());
    };
    let head = workspace.head()?;
    let stale = record.base_revision != meta.base_revision
        || record.base_manifest != meta.base_manifest
        || head.as_ref().is_none_or(|head| {
            head.revision().as_str() != record.draft_head.as_str()
                || head.manifest_digest() != record.draft_manifest
        });
    if stale {
        clear(dir)?;
        return Ok(Vec::new());
    }
    Ok(record.conflicts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_engine::footprint::Entry;
    use beamcraft_model::id::OpId;

    fn conflict(n: usize) -> Conflict {
        Conflict {
            draft: OpId::parse(&format!("019a7bf0-0000-7000-8000-{n:012x}")).unwrap(),
            accepted: OpId::parse(&format!("019a7bf0-0000-7000-8000-{n:012x}")).unwrap(),
            entry: Entry {
                target: Target::Element(
                    ElementId::parse(&format!("019a7bf0-0000-7000-8000-{n:012x}")).unwrap(),
                ),
                group: Group::Label,
            },
            kind: ConflictKind::WriteWrite,
        }
    }

    #[test]
    fn the_record_round_trips_canonically_and_refuses_other_bytes() {
        let record = PendingConflicts {
            base_revision: Uuid7::parse(&id(1)).unwrap(),
            base_manifest: "1111111111111111111111111111111111111111111111111111111111111111"
                .parse()
                .unwrap(),
            draft_head: Uuid7::parse(&id(2)).unwrap(),
            draft_manifest: "2222222222222222222222222222222222222222222222222222222222222222"
                .parse()
                .unwrap(),
            conflicts: vec![conflict(3), conflict(4)],
        };
        let text = record.encode().unwrap();
        assert_eq!(PendingConflicts::decode(&text).unwrap(), record);
        assert_eq!(record.encode().unwrap(), text);
        assert!(PendingConflicts::decode(&format!("{text}conflict=extra\n")).is_err());
        assert!(PendingConflicts::decode(&text.replace("write_write", "nonsense")).is_err());
        assert!(PendingConflicts::decode(&text.replace("draft_head", "draft_headx")).is_err());
        assert!(PendingConflicts::decode("beamcraft-conflicts 2\n").is_err());
    }

    fn id(n: usize) -> String {
        format!("019a7bf0-0000-7000-8000-{n:012x}")
    }
}
