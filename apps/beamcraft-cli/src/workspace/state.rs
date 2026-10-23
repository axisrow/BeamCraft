// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! C03.6: serialized compare-and-swap of workspace metadata, independent of draft commits.

use super::{META_CAP, META_FILE, WorkspaceMeta, meta_path, unusable};
use crate::plate::PlateError;
use beamcraft_host::lease::{StoreLease, WorkspaceLease};
use beamcraft_host::storage::BlobStore;
use beamcraft_model::error::ErrorCode;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

fn changed() -> PlateError {
    PlateError::new(ErrorCode::BaseRevisionMismatch, "workspace metadata moved")
}

fn read(dir: &Path) -> Result<Option<WorkspaceMeta>, PlateError> {
    let path = meta_path(dir);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(unusable()),
    };
    if !metadata.is_file() || metadata.len() > META_CAP {
        return Err(unusable());
    }
    let file = fs::File::open(&path).map_err(|_| unusable())?;
    let mut bytes = Vec::new();
    file.take(META_CAP.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| unusable())?;
    if bytes.len() as u64 > META_CAP {
        return Err(unusable());
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| unusable())?;
    Ok(Some(WorkspaceMeta::decode(text)?))
}

/// One locked metadata snapshot of a personal workspace. Holding it across a draft commit
/// serializes base transitions without stopping ordinary model commands or replacing their CAS.
/// It proves no accepted base pin or permissions; callers must verify those independently.
pub struct WorkspaceState {
    dir: PathBuf,
    meta: Option<WorkspaceMeta>,
    _lease: WorkspaceLease,
    _maintenance: StoreLease,
}

impl WorkspaceState {
    /// Locks the existing local directory and reads bounded metadata. `None` is an uninitialized
    /// workspace; malformed existing metadata fails. A concurrent holder returns immediately
    /// with `BACKEND_UNAVAILABLE`; no polling, stale lock deletion or external service is used.
    pub fn open(dir: &Path) -> Result<Self, PlateError> {
        let store = BlobStore::new(dir)
            .map_err(|error| PlateError::new(error.code(), "workspace directory is not usable"))?;
        let maintenance = StoreLease::shared(&store)
            .map_err(|error| PlateError::new(error.code(), "workspace is in use or not usable"))?;
        let lease = WorkspaceLease::exclusive(&store).map_err(|error| {
            PlateError::new(error.code(), "workspace state is in use or not usable")
        })?;
        let mut state = Self {
            dir: dir.to_owned(),
            meta: read(dir)?,
            _lease: lease,
            _maintenance: maintenance,
        };
        super::pending::recover(&mut state)?;
        Ok(state)
    }

    /// The complete snapshot read while acquiring this lease. An absent file stays explicit.
    pub fn meta(&self) -> Option<&WorkspaceMeta> {
        self.meta.as_ref()
    }

    pub(super) fn directory(&self) -> &Path {
        &self.dir
    }

    /// Records a bounded, synced rebase intent and returns the evidence pin for its candidate.
    /// The caller must first pin/verify the accepted basis, prove command independence and stage
    /// the complete candidate. This checks the exact original metadata, draft and source proof.
    /// The candidate must use the rebase operation as both revision and operation identity and
    /// be committed with this pin through the normal strict verifier against the original draft.
    pub fn stage_rebase(
        &self,
        receipt: &super::rebase_receipt::RebaseReceipt,
    ) -> Result<beamcraft_host::revision::EvidencePin, PlateError> {
        super::pending::stage(self, receipt)
    }

    /// Finalizes an intent from the exact committed candidate, retaining any later local edits.
    /// An absent/lost CAS slot removes an abandoned intent without changing metadata. Missing or
    /// corrupt history/evidence refuses and retains the intent. `open` also runs this recovery
    /// before exposing any workspace state. No revision or published store is changed here.
    pub fn finish_rebase(&mut self) -> Result<Option<crate::plate::RevisionSummary>, PlateError> {
        super::pending::recover(self)
    }

    /// Atomically replaces exactly `expected`, preserving workspace/user identity and never
    /// moving the already-published local prefix backwards. `None` only creates an absent file.
    /// A stale snapshot is `BASE_REVISION_MISMATCH` before any temporary file is written. Equal
    /// expected/result metadata is an unchanged success (`false`), otherwise the result is `true`.
    /// A zero rebase boundary preserves v1 bytes; a nonzero one uses canonical v2. As with the
    /// revision store, directory-entry durability is
    /// not guaranteed; the complete temporary file is synced before rename.
    pub fn compare_and_swap(
        &mut self,
        expected: Option<&WorkspaceMeta>,
        next: &WorkspaceMeta,
    ) -> Result<bool, PlateError> {
        let current = read(&self.dir)?;
        if current.as_ref() != expected || current != self.meta {
            return Err(changed());
        }
        if next.synced_sequence == 0
            || next.rebase_sequence > beamcraft_host::revision::MAX_REVISIONS
            || current.as_ref().is_some_and(|old| {
                old.workspace_id != next.workspace_id
                    || old.user_id != next.user_id
                    || next.synced_sequence < old.synced_sequence
                    || next.rebase_sequence < old.rebase_sequence
            })
        {
            return Err(PlateError::new(
                ErrorCode::InvalidArgument,
                "workspace identity, published prefix or rebase boundary cannot move backwards",
            ));
        }
        if current.as_ref() == Some(next) {
            return Ok(false);
        }
        let temporary = self.dir.join(format!("{META_FILE}.tmp"));
        // Every cooperating writer holds this lease. A leftover/planted entry is removed as an
        // entry only; it is never opened for writing through a link.
        match fs::symlink_metadata(&temporary) {
            Ok(_) => fs::remove_file(&temporary).map_err(|_| unusable())?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(unusable()),
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|_| unusable())?;
        file.write_all(next.encode().as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|_| unusable())?;
        fs::rename(&temporary, meta_path(&self.dir)).map_err(|_| unusable())?;
        self.meta = Some(next.clone());
        Ok(true)
    }
}
