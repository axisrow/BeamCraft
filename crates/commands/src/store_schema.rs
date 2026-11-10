// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The layer-schema status of a store's verified head (issue 555 finding 5).
//!
//! `open-revision` verifies the manifest and every blob, which says nothing about whether this
//! build can interpret the layers: a store written by a build with another layer mapping
//! (another `SCHEMA_DESCRIPTOR` digest) is intact yet every content command refuses it with
//! `UNSUPPORTED_VERSION`. Pre-alpha stores are deliberately not migrated, so the honest answer
//! is to report the store's pins next to this build's digest and let `open-revision` refuse an
//! incompatible store instead of calling it verified.

use beamcraft_host::revision::{RevisionError, RevisionStore};
use beamcraft_layer_engine::authored::{schema_identity, stored_schema_pins};
use beamcraft_model::id::{ProjectId, WorkspaceId};
use session::Refusal;
use session::pipeline::{integrity, load_base, open_base};

/// The layer-schema pins of a store's head against this build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaStatus {
    /// The distinct `bimSchemaDigest` pins of the head's owned layers, ascending.
    pub store_digests: Vec<String>,
    /// The digest of this build's layer mapping.
    pub build_digest: String,
    /// Whether every pin is this build's digest, so content commands can read the store.
    pub compatible: bool,
}

/// The schema status of the verified head; `None` for a store without a revision.
pub fn schema_status(store: &RevisionStore) -> Result<Option<SchemaStatus>, Refusal> {
    let revision = match store.open() {
        Ok(revision) => revision,
        Err(RevisionError::NoRevision) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let content = revision.manifest().content();
    let damaged = |_| integrity("head manifest identity is not a UUIDv7");
    let project = ProjectId::parse(content.project().as_str()).map_err(damaged)?;
    let workspace = WorkspaceId::parse(content.workspace().as_str()).map_err(damaged)?;
    let (_, base) = load_base(store, &project, &workspace)?;
    let request = open_base(&base)?;
    let store_digests = stored_schema_pins(&request)?;
    let build_digest = schema_identity()?.digest.to_string();
    let compatible = store_digests.iter().all(|pin| *pin == build_digest);
    Ok(Some(SchemaStatus {
        store_digests,
        build_digest,
        compatible,
    }))
}
