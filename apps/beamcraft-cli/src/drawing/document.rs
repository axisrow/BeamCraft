// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Closed D03 document-root contract, separate from the engineering-model schema.
//!
//! This module only encodes immutable document metadata. A later store adapter combines it
//! with the authored intent, derived projection, and exact source archive in one revision.

use crate::drawing::derived::{self, ROOT_LAYER};
use crate::plate::{PlateError, integrity};
use beamcraft_layer_engine::Asset;
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ActorId, DocumentVersionId, OpId};

/// Tight bound for the complete document root, including all persisted operation metadata.
pub const MAX_DOCUMENT_ROOT_BYTES: usize = 16_384;
/// Exact descriptor for the separate immutable document/revision contract.
pub const SCHEMA_DESCRIPTOR: &str = "beamcraft-drawing-document/1;root=Document;state=draft,published;version=immutable-uuidv7;latestPublished=optional-uuidv7;request=sha256;actor=uuidv7;operation=uuidv7;drawing-root=root.usda;source-archive=external-only";

/// The state persisted with one atomic document revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// A mutable working document; publication has not been accepted.
    Draft,
    /// An immutable published version.
    Published,
}

impl State {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Published => "published",
        }
    }
}

/// Complete, typed metadata needed to reproduce a document root byte-for-byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metadata {
    /// Current mutable/published state.
    pub state: State,
    /// Immutable version id, present exactly when state is published.
    pub version: Option<DocumentVersionId>,
    /// Most recently published immutable version, retained across later draft refreshes.
    pub latest_published: Option<DocumentVersionId>,
    /// Digest of the canonical user request accepted by the host ledger.
    pub request: ContentDigest,
    /// Recorded actor identity. This is attribution, not authorization.
    pub actor: ActorId,
    /// Host idempotency key for this exact revision.
    pub operation: OpId,
}

/// Stable schema identity for document revisions. It is never the model schema digest.
pub fn schema_identity() -> Result<ContentDigest, PlateError> {
    Sha256::digest(SCHEMA_DESCRIPTOR.as_bytes())
        .map_err(|_| integrity("document schema digest failed"))
}

fn escaped(value: &str) -> String {
    let mut result = String::with_capacity(value.len() + 2);
    result.push('"');
    for ch in value.chars() {
        match ch {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            _ => result.push(ch),
        }
    }
    result.push('"');
    result
}

fn optional(value: Option<impl ToString>) -> String {
    value.map_or_else(|| "\"\"".to_owned(), |value| escaped(&value.to_string()))
}

/// Produces the canonical composition root with state metadata and the two drawing layers.
/// Source model assets are never sublayers and are not exposed to USD composition.
pub fn encode(metadata: Metadata) -> Result<Asset, PlateError> {
    if (metadata.state == State::Published) != metadata.version.is_some()
        || metadata
            .version
            .is_some_and(|version| Some(version) != metadata.latest_published)
    {
        return Err(PlateError {
            code: ErrorCode::InvalidArgument,
            message: "published drawing version metadata is inconsistent",
        });
    }
    let root = derived::root()?;
    let insertion = format!(
        "def Scope \"Document\"\n{{\n    custom string drawing:documentSchema = \"{}\"\n    custom token drawing:state = \"{}\"\n    custom string drawing:version = {}\n    custom string drawing:latestPublished = {}\n    custom string drawing:request = \"{}\"\n    custom string drawing:actor = \"{}\"\n    custom string drawing:operation = \"{}\"\n}}\n",
        schema_identity()?,
        metadata.state.as_str(),
        optional(metadata.version),
        optional(metadata.latest_published),
        metadata.request,
        metadata.actor,
        metadata.operation,
    );
    let mut bytes = root.bytes;
    bytes
        .try_reserve(insertion.len())
        .map_err(|_| integrity("document root allocation failed"))?;
    bytes.extend_from_slice(insertion.as_bytes());
    if bytes.len() > MAX_DOCUMENT_ROOT_BYTES {
        return Err(PlateError {
            code: ErrorCode::LimitExceeded,
            message: "document root byte cap exceeded",
        });
    }
    let digest = Sha256::digest(&bytes).map_err(|_| integrity("document root digest failed"))?;
    Ok(Asset {
        name: ROOT_LAYER.to_owned(),
        digest,
        bytes,
    })
}

/// Requires persisted root bytes to be the unique canonical encoding of the supplied metadata.
pub fn verify(saved: &Asset, metadata: Metadata) -> Result<(), PlateError> {
    if saved.name != ROOT_LAYER || saved.bytes.len() > MAX_DOCUMENT_ROOT_BYTES {
        return Err(integrity("document root identity or size disagrees"));
    }
    let expected = encode(metadata)?;
    if saved.digest != expected.digest || saved.bytes != expected.bytes {
        return Err(integrity("document root metadata is not canonical"));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn metadata(state: State, version: Option<DocumentVersionId>) -> Metadata {
        Metadata {
            state,
            version,
            latest_published: version,
            request: Sha256::digest(b"request").unwrap(),
            actor: ActorId::parse("018f0f5e-7b00-7000-8000-000000000001").unwrap(),
            operation: OpId::parse("018f0f5e-7b00-7000-8000-000000000002").unwrap(),
        }
    }

    #[test]
    fn draft_root_is_separate_schema_and_composes_only_drawing_layers() {
        let root = encode(metadata(State::Draft, None)).unwrap();
        let text = std::str::from_utf8(&root.bytes).unwrap();
        assert!(text.contains("drawing:documentSchema"));
        assert!(text.contains("drawing:state = \"draft\""));
        assert!(text.contains("@drawing.usda@, @drawing-derived.usda@"));
        assert!(!text.contains("model/"));
        assert!(root.bytes.len() <= MAX_DOCUMENT_ROOT_BYTES);
    }

    #[test]
    fn published_metadata_requires_a_matching_latest_version() {
        let version = DocumentVersionId::parse("018f0f5e-7b00-7000-8000-000000000003").unwrap();
        assert!(encode(metadata(State::Published, Some(version))).is_ok());
        assert!(encode(metadata(State::Draft, Some(version))).is_err());
        let mut inconsistent = metadata(State::Published, Some(version));
        inconsistent.latest_published = None;
        assert!(encode(inconsistent).is_err());
    }

    #[test]
    fn verification_rejects_self_consistently_rehashed_metadata_tampering() {
        let original = encode(metadata(State::Draft, None)).unwrap();
        let mut altered = original.clone();
        altered.bytes.extend_from_slice(b"# untrusted\n");
        altered.digest = Sha256::digest(&altered.bytes).unwrap();
        assert!(verify(&altered, metadata(State::Draft, None)).is_err());
    }

    #[test]
    fn schema_identity_is_stable_and_distinct_from_drawing_schema() {
        assert_eq!(schema_identity().unwrap(), schema_identity().unwrap());
        assert_ne!(
            schema_identity().unwrap(),
            crate::drawing::codec::schema_identity().unwrap()
        );
    }
}
