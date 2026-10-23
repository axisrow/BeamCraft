// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Element identity and kind for the view contract.
//!
//! `ElementId` is `beamcraft_model::id::ElementId` (F04 section 2: full UUIDv7, canonical
//! lowercase); V02 held a private copy until P01 provided the model type. `ElementKind`
//! mirrors the closed F10 section 2.1 registry; it classifies, it does not imply geometry.

use crate::error::{ViewCode, ViewError};

pub use beamcraft_model::id::ElementId;
use beamcraft_model::id::IdError;

impl From<IdError> for ViewError {
    fn from(_: IdError) -> Self {
        Self::new(ViewCode::IdInvalid)
    }
}

/// The element kinds of the closed F10 registry (vocabulary 0.1).
///
/// Welds and bolts are not registered kinds yet; they enter with a later vocabulary version and
/// then become ordinary values of this enum. The kind filter works on any registered kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ElementKind {
    /// Steel plate.
    Plate,
    /// Steel beam.
    Beam,
    /// Steel column.
    Column,
    /// Wall (reserved).
    Wall,
    /// Slab (reserved).
    Slab,
    /// Opening (reserved).
    Opening,
    /// Penetration (reserved).
    Penetration,
    /// Pipe (reserved).
    Pipe,
    /// Duct (reserved).
    Duct,
    /// Support (reserved).
    Support,
    /// Assembly (reserved).
    Assembly,
    /// Connection (reserved).
    Connection,
    /// Read-only reference to foreign content.
    Reference,
}

impl ElementKind {
    /// Every kind, in registry order.
    pub const ALL: [Self; 13] = [
        Self::Plate,
        Self::Beam,
        Self::Column,
        Self::Wall,
        Self::Slab,
        Self::Opening,
        Self::Penetration,
        Self::Pipe,
        Self::Duct,
        Self::Support,
        Self::Assembly,
        Self::Connection,
        Self::Reference,
    ];

    /// The lowercase registry token stored in `bim:elementKind`.
    pub const fn as_token(self) -> &'static str {
        match self {
            Self::Plate => "plate",
            Self::Beam => "beam",
            Self::Column => "column",
            Self::Wall => "wall",
            Self::Slab => "slab",
            Self::Opening => "opening",
            Self::Penetration => "penetration",
            Self::Pipe => "pipe",
            Self::Duct => "duct",
            Self::Support => "support",
            Self::Assembly => "assembly",
            Self::Connection => "connection",
            Self::Reference => "reference",
        }
    }

    /// Parses an exact registry token.
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.as_token() == token)
    }

    const fn bit(self) -> u16 {
        1 << (self as u16)
    }
}

/// The set of element kinds a view hides. The default hides nothing.
///
/// It is view state: it belongs to user or workspace settings, never to the shared revision, and
/// it changes no authored or derived data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct KindFilter {
    hidden: u16,
}

impl KindFilter {
    /// A filter that hides nothing.
    pub const SHOW_ALL: Self = Self { hidden: 0 };

    /// The filter with `kind` additionally hidden.
    pub const fn hiding(self, kind: ElementKind) -> Self {
        Self {
            hidden: self.hidden | kind.bit(),
        }
    }

    /// The filter with `kind` shown again.
    pub const fn showing(self, kind: ElementKind) -> Self {
        Self {
            hidden: self.hidden & !kind.bit(),
        }
    }

    /// Whether elements of `kind` are shown.
    pub const fn is_visible(self, kind: ElementKind) -> bool {
        self.hidden & kind.bit() == 0
    }

    /// The hidden kinds in registry order.
    pub fn hidden_kinds(self) -> Vec<ElementKind> {
        ElementKind::ALL
            .into_iter()
            .filter(|k| !self.is_visible(*k))
            .collect()
    }

    /// Parses a comma-separated list of registry tokens, for example `plate,support`.
    ///
    /// The text is at most 256 bytes; an unknown or empty token is `VIEW_ARGUMENT_INVALID`.
    pub fn parse_list(text: &str) -> Result<Self, ViewError> {
        let invalid = || ViewError::new(ViewCode::ArgumentInvalid);
        if text.len() > 256 {
            return Err(invalid());
        }
        let mut filter = Self::SHOW_ALL;
        for token in text.split(',') {
            filter = filter.hiding(ElementKind::parse(token).ok_or_else(invalid)?);
        }
        Ok(filter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "01990000-0000-7000-8000-000000000001";

    #[test]
    fn parses_and_prints_the_canonical_form() {
        let id = ElementId::parse(GOOD).unwrap();
        assert_eq!(id.to_string(), GOOD);
        assert_eq!(id.as_bytes().len(), 16);
        let later = ElementId::parse("01990000-0000-7000-8000-00000000000a").unwrap();
        assert!(id < later);
    }

    #[test]
    fn rejects_every_malformed_id() {
        for bad in [
            "",
            "01990000-0000-7000-8000-00000000000",
            "01990000-0000-7000-8000-0000000000012",
            "01990000_0000-7000-8000-000000000001",
            "01990000-0000-7000-8000-00000000000A",
            "01990000-0000-7000-8000-00000000000g",
            "01990000-0000-4000-8000-000000000001",
            "01990000-0000-7000-c000-000000000001",
            "01990000-0000-7000-7000-000000000001",
            "{01990000-0000-7000-8000-000000000001}",
            "０1990000-0000-7000-8000-000000000001",
        ] {
            assert_eq!(
                ViewError::from(ElementId::parse(bad).unwrap_err()).code(),
                ViewCode::IdInvalid,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn kinds_round_trip_and_are_unique() {
        for (i, kind) in ElementKind::ALL.iter().enumerate() {
            assert_eq!(ElementKind::parse(kind.as_token()), Some(*kind));
            assert_eq!(*kind as usize, i);
        }
        assert_eq!(ElementKind::parse("Plate"), None);
        assert_eq!(ElementKind::parse("weld"), None);
    }

    #[test]
    fn filter_hides_shows_and_parses() {
        let f = KindFilter::default();
        assert!(ElementKind::ALL.iter().all(|k| f.is_visible(*k)));
        let f = f
            .hiding(ElementKind::Support)
            .hiding(ElementKind::Reference);
        assert!(!f.is_visible(ElementKind::Support));
        assert!(f.is_visible(ElementKind::Plate));
        assert_eq!(
            f.hidden_kinds(),
            vec![ElementKind::Support, ElementKind::Reference]
        );
        assert!(
            f.showing(ElementKind::Support)
                .is_visible(ElementKind::Support)
        );
        assert_eq!(KindFilter::parse_list("support,reference").unwrap(), f);
        for bad in ["", "plate,", "weld", "Plate", &"plate,".repeat(60)] {
            assert_eq!(
                KindFilter::parse_list(bad).unwrap_err().code(),
                ViewCode::ArgumentInvalid,
                "{bad:?}"
            );
        }
    }
}
