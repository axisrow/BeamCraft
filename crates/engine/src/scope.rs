// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Authored section and zone scopes (U19, F10 0.2 `DV-SCOPE-001..004`, F12 section 3).
//!
//! A **section** (for example `KM` or `KMD`) is an independently responsible project
//! deliverable; a **zone** (for example `K1`) is an optional project partition. Both are named
//! here by bounded codes; under F12 the full identities are UUIDv7 records of the project's
//! section and zone registries, which this slice does not implement (the codes are the labels
//! the authored layer names and the workspace authorization are built from until then).
//!
//! A module (`steel`) is neither: KM and KMD can both be written through the same steel module
//! while remaining separate writable sections, and enabling a module never widens a scope.
//!
//! The authored layer of one section/zone scope is named `<code>[_<zone>]_<section>.usda` with
//! the project code, and the partition's grids and levels live in
//! `<code>[_<zone>]_Grids.usda` (T01 §5.2/§6). A code feeds a file name, so its charset is
//! restricted to ASCII
//! alphanumerics and `-`; the F12 registries may widen the label rules for display names, which
//! never enter a layer name.

use std::collections::BTreeSet;

use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;

/// Longest project, section or zone code, in bytes.
pub const MAX_CODE_BYTES: usize = 32;

/// `DV-SCOPE-001`: the workspace scope is authorized by the trusted context, never claimed.
pub const RULE_SCOPE_AUTHORIZED: &str = "DV-SCOPE-001";
/// `DV-SCOPE-002`: an element edit stays inside the active section and allowed zone.
pub const RULE_ELEMENT_IN_SCOPE: &str = "DV-SCOPE-002";
/// `DV-SCOPE-003`: publication preserves other sections and the root composition.
pub const RULE_PRESERVE_FOREIGN: &str = "DV-SCOPE-003";
/// `DV-SCOPE-004`: shared metadata changes only through authorized shared commands.
pub const RULE_SHARED_AUTHORIZED: &str = "DV-SCOPE-004";

const fn invalid(message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::InvalidArgument, "scope", message)
}

const fn refused(message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::PermissionDenied, "scope", message)
}

/// Validates one project, section or zone code: 1 to [`MAX_CODE_BYTES`] bytes, ASCII
/// alphanumerics and `-`, so the code can enter a layer file name on every platform.
pub fn check_code(code: &str) -> Result<(), CommandError> {
    if code.is_empty()
        || code.len() > MAX_CODE_BYTES
        || !code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(invalid(
            "a project, section or zone code must be 1 to 32 ASCII letters, digits or hyphens",
        ));
    }
    Ok(())
}

/// The authored layer name of one section/zone scope: `<code>[_<zone>]_<section>.usda`.
pub fn section_layer(
    code: &str,
    zone: Option<&str>,
    section: &str,
) -> Result<String, CommandError> {
    check_code(code)?;
    check_code(section)?;
    if let Some(zone) = zone {
        check_code(zone)?;
        Ok(format!("{code}_{zone}_{section}.usda"))
    } else {
        Ok(format!("{code}_{section}.usda"))
    }
}

/// The grids/levels layer of one zone partition (T01 §5.2/§6): the unzoned partition keeps the
/// common grids in `<code>_Grids.usda`, a zone partition keeps its own grids and levels in
/// `<code>_<zone>_Grids.usda`. The zone of a grid or level is the layer it is stored in;
/// sections reference the layer without gaining edit rights.
pub fn grids_layer(code: &str, zone: Option<&str>) -> Result<String, CommandError> {
    check_code(code)?;
    if let Some(zone) = zone {
        check_code(zone)?;
        return Ok(format!("{code}_{zone}_Grids.usda"));
    }
    Ok(format!("{code}_Grids.usda"))
}

/// One workspace's authorized write scope: one active section, an explicit zone partition and
/// optionally the separately authorized shared grids/levels scope (`DV-SCOPE-001`).
///
/// The zone set is a set because the contract states it as one; this slice authorizes at most
/// one partition, because element creation needs one unambiguous write layer. A wider zone set
/// arrives when element commands name their zone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceScope {
    /// Project code the layer names are built from.
    pub code: String,
    /// The active section code.
    pub section: String,
    /// The authorized zone partition; empty authors the unzoned partition.
    pub zones: BTreeSet<String>,
    /// Whether the shared grids/levels scope is separately authorized (`DV-SCOPE-004`).
    pub shared: bool,
}

impl WorkspaceScope {
    /// Validates the codes and the single-partition rule.
    pub fn validate(&self) -> Result<(), CommandError> {
        check_code(&self.code)?;
        check_code(&self.section)?;
        if self.zones.len() > 1 {
            return Err(refused(
                "a workspace is authorized for one zone partition, because element creation \
                 needs one write layer",
            ));
        }
        for zone in &self.zones {
            check_code(zone)?;
        }
        Ok(())
    }

    /// The zone partition as an `Option`, the empty set meaning the unzoned partition.
    pub fn zone(&self) -> Option<&str> {
        self.zones.iter().next().map(String::as_str)
    }

    /// The authored layer of the active section/zone scope: the only layer ordinary element
    /// edits may change.
    pub fn section_layer(&self) -> Result<String, CommandError> {
        section_layer(&self.code, self.zone(), &self.section)
    }

    /// The grids/levels layer of the authorized partition: the common layer for the unzoned
    /// partition, the zone's own layer otherwise (T01 §6).
    pub fn grids_layer(&self) -> Result<String, CommandError> {
        grids_layer(&self.code, self.zone())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn codes_feed_portable_layer_names() {
        assert_eq!(
            section_layer("2401-DEMO", Some("K1"), "KMD").unwrap(),
            "2401-DEMO_K1_KMD.usda"
        );
        assert_eq!(
            section_layer("2401-DEMO", None, "KM").unwrap(),
            "2401-DEMO_KM.usda"
        );
        assert_eq!(
            grids_layer("2401-DEMO", None).unwrap(),
            "2401-DEMO_Grids.usda"
        );
        assert_eq!(
            grids_layer("2401-DEMO", Some("K1")).unwrap(),
            "2401-DEMO_K1_Grids.usda"
        );
        assert!(grids_layer("2401-DEMO", Some("K_1")).is_err());
        for bad in ["", "a_b", "a.b", "a/b", "a b", "КМ", &"x".repeat(33)] {
            assert!(check_code(bad).is_err(), "{bad:?}");
        }
        assert!(check_code("KM").is_ok() && check_code("2401-DEMO").is_ok());
    }

    #[test]
    fn a_scope_names_its_layers_and_validates() {
        let scope = WorkspaceScope {
            code: "2401-DEMO".into(),
            section: "KMD".into(),
            zones: BTreeSet::from(["K1".to_owned()]),
            shared: false,
        };
        scope.validate().unwrap();
        assert_eq!(scope.section_layer().unwrap(), "2401-DEMO_K1_KMD.usda");
        assert_eq!(scope.grids_layer().unwrap(), "2401-DEMO_K1_Grids.usda");
        let unzoned = WorkspaceScope {
            zones: BTreeSet::new(),
            ..scope
        };
        assert_eq!(unzoned.section_layer().unwrap(), "2401-DEMO_KMD.usda");
        assert_eq!(unzoned.grids_layer().unwrap(), "2401-DEMO_Grids.usda");
        let multi = WorkspaceScope {
            zones: BTreeSet::from(["K1".to_owned(), "K2".to_owned()]),
            ..unzoned
        };
        assert_eq!(
            multi.validate().unwrap_err().code,
            ErrorCode::PermissionDenied
        );
        let bad = WorkspaceScope {
            section: "K_M".into(),
            ..multi
        };
        assert_eq!(bad.validate().unwrap_err().code, ErrorCode::InvalidArgument);
    }
}
