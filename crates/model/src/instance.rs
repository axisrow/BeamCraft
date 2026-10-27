// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! P11 instance copy requests. Independent duplication remains `CopyElement`.
use crate::{
    command::{CommandError, CopyElement},
    digest::{ContentDigest, Sha256},
    error::ErrorCode,
    id::ElementId,
};

/// Most linked part instances in one model, keeping an atomic group update within footprint limits.
pub const MAX_PART_INSTANCES: usize = 256;

/// Supported kind of a shared part definition and its instances.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartKind {
    /// Plate definition.
    Plate,
    /// Straight beam definition.
    Beam,
    /// Straight column definition.
    Column,
}
impl PartKind {
    /// Exact authored kind token.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Plate => "plate",
            Self::Beam => "beam",
            Self::Column => "column",
        }
    }
    /// Recognizes a supported part kind.
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "plate" => Some(Self::Plate),
            "beam" => Some(Self::Beam),
            "column" => Some(Self::Column),
            _ => None,
        }
    }
}

/// A new part that shares its source's shape, profile, material and operation definition.
#[derive(Debug, Clone, PartialEq)]
pub struct CopyInstance {
    /// Same identity, label and placement-offset fields as independent duplication.
    pub copy: CopyElement,
}
impl CopyInstance {
    /// Admits IDs, label and offset using the existing bounded copy argument rules.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        self.copy.validate_arguments()
    }
    /// Distinguishes the instance operation from an independent duplicate with the same arguments.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        let mut bytes = b"BeamCraft/P11/CopyInstance/v1\0".to_vec();
        bytes.extend_from_slice(self.copy.content_fingerprint()?.as_bytes());
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(
                ErrorCode::LimitExceeded,
                "command",
                "instance request exceeds digest limit",
            )
        })
    }
}

/// A persistent relation to the canonical part that owns a shared definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstanceLink {
    /// Part with its own identity, placement, label, level and relationships.
    pub instance: ElementId,
    /// Canonical part owning shape, profile, material and operations. Never another instance.
    pub definition: ElementId,
}
