// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Stable USD categories and paths (architecture, Part 3; U16).

use crate::digest::ContentDigest;

/// An element's immutable path category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementCategory {
    /// Plate, beam or column.
    Part,
    /// Assembly of parts or assemblies.
    Assembly,
    /// Weld or bolt group.
    Connection,
    /// Shared grid.
    Grid,
    /// Shared level.
    Level,
    /// T02 construction object: a helper point, line, circle or plane.
    Construction,
    /// Read-only foreign model wrapper.
    Reference,
}

impl ElementCategory {
    /// Every supported element category.
    pub const ALL: [Self; 7] = [
        Self::Part,
        Self::Assembly,
        Self::Connection,
        Self::Level,
        Self::Grid,
        Self::Construction,
        Self::Reference,
    ];

    /// Maps `bim:elementKind` to its path category.
    pub fn from_kind(kind: &str) -> Option<Self> {
        match kind {
            "plate" | "beam" | "column" => Some(Self::Part),
            "assembly" => Some(Self::Assembly),
            "connection" => Some(Self::Connection),
            "level" => Some(Self::Level),
            "grid" => Some(Self::Grid),
            "construction" => Some(Self::Construction),
            "reference" => Some(Self::Reference),
            _ => None,
        }
    }

    /// Canonical category folder and three-letter prefix, including the trailing underscore.
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Part => "/World/Steel/Parts/PRT_",
            Self::Assembly => "/World/Steel/Assemblies/ASM_",
            Self::Connection => "/World/Steel/Connections/CON_",
            Self::Level => "/World/Levels/LVL_",
            Self::Grid => "/World/Grids/GRD_",
            Self::Construction => "/World/Construction/CNS_",
            Self::Reference => "/World/References/REF_",
        }
    }
}

/// Immutable definition category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefinitionCategory {
    /// Engineering material, separate from display appearance.
    Material,
    /// Section profile of any supported family.
    Profile,
    /// Fastener definition (A14.1, bolt amendment section 2).
    Fastener,
}

impl DefinitionCategory {
    /// Canonical category folder and prefix.
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Material => "/World/Definitions/Materials/MAT_",
            Self::Profile => "/World/Definitions/Profiles/PRF_",
            Self::Fastener => "/World/Definitions/Fasteners/FST_",
        }
    }

    /// Full canonical path of a definition.
    pub fn path(self, digest: ContentDigest) -> String {
        format!("{}{digest}", self.prefix())
    }

    /// Parses an exact category path with a full lowercase SHA-256; descendants are excluded.
    pub fn parse_path(path: &str) -> Option<(Self, ContentDigest)> {
        [Self::Material, Self::Profile, Self::Fastener]
            .into_iter()
            .find_map(|category| {
                Some((
                    category,
                    path.strip_prefix(category.prefix())?.parse().ok()?,
                ))
            })
    }

    /// The category the `bim:definitionKind` token names: `material` is a material, `fastener` a
    /// fastener (A14.1), every other admitted kind a section profile.
    pub fn for_kind(kind: &str) -> Self {
        match kind {
            "material" => Self::Material,
            "fastener" => Self::Fastener,
            _ => Self::Profile,
        }
    }
}
