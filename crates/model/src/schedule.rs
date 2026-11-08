// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! I05 stored schedule templates: the command inputs that save an admitted template into the
//! model as a shared, owned definition (`/World/Definitions/Schedules/<name>`), so a schedule
//! carries revisions, footprints, commit and undo like any authored entity. The template
//! travels as its version-1 JSON document; its admission against the attribute state is the
//! commands layer's decision, made before the change is authored.

use crate::attribute::attr_name_is_valid;
use crate::command::CommandError;
use crate::digest::{ContentDigest, Sha256};
use crate::error::ErrorCode;

/// The largest stored template document, the same bound the file form admits.
pub const TEMPLATE_MAX_BYTES: usize = 65_536;

fn invalid(message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::InvalidArgument, "name", message)
}

/// Whether `name` is a valid schedule-template name: the same form as an attribute name, so
/// the definition prim carries it verbatim.
pub fn schedule_name_is_valid(name: &str) -> bool {
    attr_name_is_valid(name)
}

/// I05 `DefineSchedule`: stores one schedule template under its name. Whether the name is
/// free is the store's decision, made when the change is decided.
#[derive(Debug, Clone, PartialEq)]
pub struct DefineSchedule {
    /// The template name.
    pub name: String,
    /// The template's version-1 JSON document, admitted before the change is authored.
    pub template: String,
}

impl DefineSchedule {
    /// Admits the name and the document bound; the template's semantics are the commands
    /// layer's decision against the pinned attribute state.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        if !schedule_name_is_valid(&self.name) {
            return Err(invalid(
                "a schedule name is 1 to 64 ASCII letters, digits or underscores",
            ));
        }
        if self.template.is_empty() || self.template.len() > TEMPLATE_MAX_BYTES {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "template",
                "the schedule template exceeds its document bound",
            ));
        }
        Ok(())
    }

    /// Versioned, length-prefixed fingerprint of the validated request.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let mut out = b"BeamCraft/I05/DefineSchedule/v1\0".to_vec();
        let put = |out: &mut Vec<u8>, s: &str| {
            out.extend_from_slice(&(s.len() as u64).to_le_bytes());
            out.extend_from_slice(s.as_bytes());
        };
        put(&mut out, &self.name);
        put(&mut out, &self.template);
        Sha256::digest(&out).map_err(|_| {
            CommandError::new(
                ErrorCode::LimitExceeded,
                "template",
                "the schedule request exceeds the encoding cap",
            )
        })
    }
}

/// I05 `UndefineSchedule`: removes one stored template. Nothing references a template
/// structurally, so an undefine has no blockers; exports naming it refuse afterwards.
#[derive(Debug, Clone, PartialEq)]
pub struct UndefineSchedule {
    /// The template name.
    pub name: String,
}

impl UndefineSchedule {
    /// Admits the name form; whether the template exists is the store's decision.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        if !schedule_name_is_valid(&self.name) {
            return Err(invalid(
                "a schedule name is 1 to 64 ASCII letters, digits or underscores",
            ));
        }
        Ok(())
    }

    /// Versioned, length-prefixed fingerprint of the validated request.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let mut out = b"BeamCraft/I05/UndefineSchedule/v1\0".to_vec();
        out.extend_from_slice(&(self.name.len() as u64).to_le_bytes());
        out.extend_from_slice(self.name.as_bytes());
        Sha256::digest(&out).map_err(|_| {
            CommandError::new(
                ErrorCode::LimitExceeded,
                "name",
                "the schedule request exceeds the encoding cap",
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_define_admits_a_valid_name_and_document() {
        let command = DefineSchedule {
            name: "frame_bom".to_owned(),
            template: r#"{"version":1,"columns":[{"column":"mark"}]}"#.to_owned(),
        };
        command.validate_arguments().unwrap();
        command.content_fingerprint().unwrap();
    }

    #[test]
    fn a_define_refuses_an_invalid_name_and_an_oversized_document() {
        let bad_name = DefineSchedule {
            name: "no dashes".to_owned(),
            template: "{}".to_owned(),
        };
        assert!(bad_name.validate_arguments().is_err());
        let oversized = DefineSchedule {
            name: "ok".to_owned(),
            template: "x".repeat(TEMPLATE_MAX_BYTES + 1),
        };
        assert!(oversized.validate_arguments().is_err());
        let empty = DefineSchedule {
            name: "ok".to_owned(),
            template: String::new(),
        };
        assert!(empty.validate_arguments().is_err());
    }

    #[test]
    fn an_undefine_admits_the_name_form() {
        let command = UndefineSchedule {
            name: "frame_bom".to_owned(),
        };
        command.validate_arguments().unwrap();
        command.content_fingerprint().unwrap();
        let bad = UndefineSchedule {
            name: String::new(),
        };
        assert!(bad.validate_arguments().is_err());
    }
}
