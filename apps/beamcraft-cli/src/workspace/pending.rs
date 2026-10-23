// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Resolve the gap between one verified draft CAS and its metadata transition.

use super::{
    WorkspaceMeta, WorkspaceState,
    rebase_receipt::{RebaseReceipt, RevisionPin},
};
use crate::plate::{PlateError, RevisionSummary, integrity, summarize};
use beamcraft_host::revision::{EvidencePin, MAX_OPERATION_EVIDENCE_BYTES, RevisionStore};
use beamcraft_host::storage::BlobStore;
use beamcraft_model::digest::Sha256;
use beamcraft_model::error::ErrorCode;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

const FILE: &str = "rebase.pending";

fn bad() -> PlateError {
    integrity("pending workspace rebase is not usable")
}

fn read(dir: &Path) -> Result<Option<Vec<u8>>, PlateError> {
    let path = dir.join(FILE);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(metadata) if metadata.is_file() && metadata.len() <= MAX_OPERATION_EVIDENCE_BYTES => {}
        _ => return Err(bad()),
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|_| bad())?
        .take(MAX_OPERATION_EVIDENCE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| bad())?;
    if bytes.len() as u64 > MAX_OPERATION_EVIDENCE_BYTES {
        return Err(bad());
    }
    Ok(Some(bytes))
}

fn store(dir: &Path) -> Result<RevisionStore, PlateError> {
    Ok(RevisionStore::new(BlobStore::new(dir).map_err(|_| bad())?))
}

fn verify_source(store: &RevisionStore, receipt: &RebaseReceipt) -> Result<(), PlateError> {
    let info = store.info_at(receipt.draft.sequence)?;
    if RevisionPin::from_info(&info) != receipt.draft
        || info.manifest().content().project() != &receipt.project
        || info.manifest().content().workspace() != &receipt.old_meta.workspace_id
        || info.manifest().content().schema() != receipt.schema
    {
        return Err(bad());
    }
    let source = crate::command_evidence::aggregate(
        store,
        receipt.old_meta.synced_sequence,
        &receipt.operations,
    )?;
    if source.head.manifest_digest() != receipt.draft.manifest
        || source.footprint.as_ref() != Some(&receipt.footprint)
    {
        return Err(bad());
    }
    Ok(())
}

fn next_meta(receipt: &RebaseReceipt) -> Result<WorkspaceMeta, PlateError> {
    let sequence = receipt.draft.sequence.checked_add(1).ok_or_else(bad)?;
    Ok(WorkspaceMeta {
        base_revision: receipt.accepted.revision.clone(),
        base_manifest: receipt.accepted.manifest,
        rebase_sequence: sequence,
        ..receipt.old_meta.clone()
    })
}

pub(super) fn stage(
    state: &WorkspaceState,
    receipt: &RebaseReceipt,
) -> Result<EvidencePin, PlateError> {
    if state.meta() != Some(&receipt.old_meta) {
        return Err(PlateError::new(
            ErrorCode::BaseRevisionMismatch,
            "workspace metadata moved",
        ));
    }
    let bytes = receipt.encode()?;
    let dir = state.directory();
    let store = store(dir)?;
    verify_source(&store, receipt)?;
    let head = store.head()?.ok_or_else(bad)?;
    if RevisionPin::from_info(&head) != receipt.draft {
        return Err(PlateError::new(
            ErrorCode::BaseRevisionMismatch,
            "draft moved before rebase intent",
        ));
    }
    let (digest, _) = store.blobs().publish(&bytes).map_err(|_| bad())?;
    let pin = EvidencePin::new(digest, bytes.len() as u64)?;
    if let Some(previous) = read(dir)? {
        return if previous == bytes {
            Ok(pin)
        } else {
            Err(bad())
        };
    }
    let temporary = dir.join("rebase.pending.tmp");
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
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| bad())?;
    fs::rename(&temporary, dir.join(FILE)).map_err(|_| bad())?;
    Ok(pin)
}

pub(super) fn recover(state: &mut WorkspaceState) -> Result<Option<RevisionSummary>, PlateError> {
    let dir = state.directory().to_owned();
    let Some(bytes) = read(&dir)? else {
        return Ok(None);
    };
    let receipt = RebaseReceipt::decode(&bytes).map_err(|_| bad())?;
    let store = store(&dir)?;
    verify_source(&store, &receipt)?;
    let head = store.head()?.ok_or_else(bad)?;
    let sequence = receipt.draft.sequence.checked_add(1).ok_or_else(bad)?;
    if head.sequence() < receipt.draft.sequence {
        return Err(bad());
    }
    let candidate = if head.sequence() >= sequence {
        Some(store.open_at(sequence)?)
    } else {
        None
    };
    let committed = candidate.as_ref().is_some_and(|revision| {
        revision.info().revision().as_str() == receipt.op_id.to_string()
            || revision.manifest().content().op().as_str() == receipt.op_id.to_string()
    });
    if !committed {
        if state.meta() != Some(&receipt.old_meta) {
            return Err(bad());
        }
        // The exact CAS slot is absent or occupied by another immutable winner. A candidate
        // based on the original draft cannot have committed later in this linear history.
        fs::remove_file(dir.join(FILE)).map_err(|_| bad())?;
        return Ok(None);
    }
    let candidate = candidate.ok_or_else(bad)?;
    let content = candidate.manifest().content();
    let expected_evidence = EvidencePin::new(
        Sha256::digest(&bytes).map_err(|_| bad())?,
        bytes.len() as u64,
    )?;
    if content.revision().as_str() != receipt.op_id.to_string()
        || content.op() != content.revision()
        || content.project() != &receipt.project
        || content.workspace() != &receipt.old_meta.workspace_id
        || content.schema() != receipt.schema
        || candidate.manifest().index().is_none()
        || content.operation_evidence() != Some(expected_evidence)
        || candidate.operation_evidence()?.as_deref() != Some(bytes.as_slice())
        || candidate.manifest().base().is_none_or(|base| {
            base.revision != receipt.draft.revision || base.manifest != receipt.draft.manifest
        })
    {
        return Err(bad());
    }
    let next = next_meta(&receipt)?;
    if state.meta() != Some(&next) {
        state.compare_and_swap(Some(&receipt.old_meta), &next)?;
    }
    fs::remove_file(dir.join(FILE)).map_err(|_| bad())?;
    Ok(Some(summarize(candidate.info())))
}

#[cfg(test)]
#[path = "pending_tests.rs"]
mod tests;
