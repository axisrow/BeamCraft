// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D01 drawing identity of the captured source model and its stale states.
//!
//! A drawing captures one exact engineering-model revision (the pin) and later compares it
//! against a live model context. Comparing pins is the whole freshness rule: it never refreshes
//! intent, changes an old published version or guesses which part of the model moved. The
//! referenced geometry itself is addressed by typed `FeatureId` references at the engine layer,
//! never by mesh indices.

use crate::digest::ContentDigest;
use crate::id::{ProjectId, RevisionId};

/// Exact identity of the captured engineering model. A document revision is a separate identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceModelPin {
    /// Source project, checked against the captured manifest by the storage adapter.
    pub project: ProjectId,
    /// Immutable source revision UUID, not the drawing revision UUID.
    pub revision: RevisionId,
    /// SHA-256 of the complete canonical source manifest.
    pub manifest: ContentDigest,
    /// Engineering-model schema digest, not the drawing schema digest.
    pub schema: ContentDigest,
    /// Exact source workspace generation.
    pub generation: u64,
}

/// Whether an application supplied a usable current-model context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveSource {
    /// No live context was requested, as when opening a self-contained historical drawing.
    Unknown,
    /// A requested live model cannot currently be verified/opened.
    Unavailable,
    /// Exact verified current model pin.
    Available(SourceModelPin),
}

/// Freshness compares pins only; it neither refreshes intent nor changes an old published version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// Captured and current pins are equal in every field.
    Current,
    /// A usable current pin differs from the captured pin.
    Stale,
    /// The requested live context could not be verified.
    Unavailable,
    /// No current-model context was supplied.
    Unknown,
}

impl SourceModelPin {
    /// Compares the exact pin without treating missing live context as current.
    pub fn freshness(self, live: LiveSource) -> Freshness {
        match live {
            LiveSource::Unknown => Freshness::Unknown,
            LiveSource::Unavailable => Freshness::Unavailable,
            LiveSource::Available(current) if current == self => Freshness::Current,
            LiveSource::Available(_) => Freshness::Stale,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(seed: u8) -> SourceModelPin {
        SourceModelPin {
            project: ProjectId::parse("01990000-0000-7000-8000-000000000001").unwrap(),
            revision: RevisionId::parse("01990000-0000-7000-8000-000000000002").unwrap(),
            manifest: ContentDigest::from_bytes([seed; 32]),
            schema: ContentDigest::from_bytes([0; 32]),
            generation: u64::from(seed),
        }
    }

    #[test]
    fn freshness_compares_the_whole_pin_without_fabricating_current() {
        let captured = pin(1);
        assert_eq!(captured.freshness(LiveSource::Unknown), Freshness::Unknown);
        assert_eq!(
            captured.freshness(LiveSource::Unavailable),
            Freshness::Unavailable
        );
        assert_eq!(
            captured.freshness(LiveSource::Available(pin(1))),
            Freshness::Current
        );
        assert_eq!(
            captured.freshness(LiveSource::Available(pin(2))),
            Freshness::Stale
        );
        // A different generation alone is already stale; nothing is patched up in place.
        let mut moved = pin(1);
        moved.generation = 2;
        assert_eq!(
            captured.freshness(LiveSource::Available(moved)),
            Freshness::Stale
        );
    }
}
