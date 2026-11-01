// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

use beamcraft_host::revision::{ContentRejection, RevisionError};
use beamcraft_layer_engine::Diagnostic;
use beamcraft_model::ErrorCode;
use beamcraft_model::command::CommandError;

/// A typed failure: a stable F04 code and a fixed safe message. No paths, no OS text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refusal {
    /// F10 rule for a domain validation finding, absent for other failures.
    pub rule_id: Option<&'static str>,
    /// Stable F04 code.
    pub code: ErrorCode,
    /// Fixed ASCII explanation.
    pub message: &'static str,
}

impl Refusal {
    /// A refusal with a code and a fixed message.
    #[must_use]
    pub const fn new(code: ErrorCode, message: &'static str) -> Self {
        Self {
            rule_id: None,
            code,
            message,
        }
    }

    /// An `INVALID_ARGUMENT` refusal.
    #[must_use]
    pub const fn invalid(message: &'static str) -> Self {
        Self::new(ErrorCode::InvalidArgument, message)
    }
}

impl From<Diagnostic> for Refusal {
    fn from(error: Diagnostic) -> Self {
        Self {
            rule_id: error.rule_id,
            code: error.code,
            message: error.message,
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Refusal {}

impl From<CommandError> for Refusal {
    fn from(error: CommandError) -> Self {
        Self {
            rule_id: error.rule_id,
            code: error.code,
            message: error.message,
        }
    }
}

impl From<ContentRejection> for Refusal {
    fn from(error: ContentRejection) -> Self {
        Self::new(error.code, error.message)
    }
}

impl From<RevisionError> for Refusal {
    fn from(error: RevisionError) -> Self {
        match error {
            RevisionError::Content(rejection) => rejection.into(),
            other => Self::new(other.code(), "revision store rejected the operation"),
        }
    }
}
