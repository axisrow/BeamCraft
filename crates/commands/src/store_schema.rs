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

use beamcraft_host::revision::Revision;
use beamcraft_layer_engine::authored::{schema_identity, stored_schema_pins};
use session::Refusal;
use session::pipeline::{ROOT_LAYER, base_with_layers, open_base};

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

/// The schema status of a verified head, read from `layers`: its verified layer bytes in
/// manifest entry order (what `RevisionStore::open_with_layers` returns), so no blob is read
/// or hashed again.
///
/// `Ok(None)` only for a head that is not a project base: its root layer is not the project
/// root (`root.usda`), as for a raw revision of arbitrary blobs; no content command reads it
/// and it has no layer schema to compare. A project head whose layers cannot be read (a
/// damaged layer committed through `commit-revision`) is an `Err` with the reader's code, the
/// same refusal the content commands give, never a silent `None`.
pub fn schema_status(
    revision: &Revision,
    layers: Vec<Vec<u8>>,
) -> Result<Option<SchemaStatus>, Refusal> {
    if revision.manifest().content().root_layer() != ROOT_LAYER {
        return Ok(None);
    }
    let base = base_with_layers(revision, layers)?;
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
