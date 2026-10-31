// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Vocabulary 0.2.0 coordination records (F10 0.2, F12 sections 2 to 4): the typed command
//! envelopes and bounded admission for section, zone, authored-provenance, project-metadata and
//! opening-decision records. The exact USD serialization and its schema pin live in the
//! layer-engine coordination module; these types are the only shapes it may store or read.
//! No authority is implied by any field: codes and names are labels (PC-ID-001).

use crate::command::CommandError;
use crate::digest::ContentDigest;
use crate::error::ErrorCode;
use crate::limits::MAX_LABEL_BYTES;

/// The vocabulary version this module admits, from F10 0.2 section 4. A record set is
/// interpreted only when the store pin names the 0.2 mapping digest; the version alone
/// never upgrades anything (DV-SCOPE-005, PC-VER-001).
pub const VOCABULARY_VERSION: &str = "0.2.0";

/// Sections per project registry (F12 section 9 budget).
pub const MAX_SECTIONS: usize = 1024;
/// Zones per project registry (F12 section 9 budget).
pub const MAX_ZONES: usize = 1024;
/// Decision records per project (F12 section 9 budget).
pub const MAX_DECISIONS: usize = 4096;
/// Registry code length cap; codes such as `KM`, `KMD`, `AR`, `OV`, `K1` are labels.
pub const MAX_CODE_BYTES: usize = 32;
/// Permitted-module tokens per section (F12 section 2).
pub const MAX_SECTION_MODULES: usize = 16;

/// Whether `code` is a registry code (PC-ID-001): 1 to 32 bytes of letters, digits, `_` and
/// `-`, starting with a letter or digit. A code is a label; it never grants or implies access.
pub fn code_is_valid(code: &str) -> bool {
    let mut chars = code.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphanumeric() => (),
        _ => return false,
    }
    code.len() <= MAX_CODE_BYTES && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Whether `token` is a module or namespace token (F10 0.1 section 4.1).
fn module_token_is_valid(token: &str) -> bool {
    let mut chars = token.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() => (),
        _ => return false,
    }
    token.len() <= 32 && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn label(name: &str, field: &'static str) -> Result<(), CommandError> {
    if name.len() > MAX_LABEL_BYTES {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            field,
            "label exceeds 256 bytes",
        ));
    }
    if name.is_empty() || name.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}') {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            field,
            "label must be 1 to 256 bytes without control characters",
        ));
    }
    Ok(())
}

fn id(value: &str, field: &'static str) -> Result<(), CommandError> {
    crate::id::Uuid7::parse(value).map(|_| ()).map_err(|_| {
        CommandError::new(
            ErrorCode::InvalidId,
            field,
            "identity must be a full UUIDv7",
        )
    })
}

/// One independently responsible project deliverable (F10 0.2 section 1, F12 section 2):
/// a stable SectionId with label fields and the module vocabulary permitted for it. KM and
/// KMD may both list `steel`; listing a module grants no cross-section access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionRecord {
    /// Stable full UUIDv7 SectionId (PC-ID-001).
    pub id: String,
    /// Human-readable code, a label only.
    pub code: String,
    /// Human-readable label.
    pub label: String,
    /// Module tokens this section's deliverables may use, 0 to 16, unique. Empty means the
    /// section is registered without a permitted module yet.
    pub modules: Vec<String>,
}

impl SectionRecord {
    /// Bounded admission (F12 section 9).
    pub fn validate(&self) -> Result<(), CommandError> {
        id(&self.id, "section_id")?;
        code(&self.code, "section_code")?;
        label(&self.label, "section_label")?;
        if self.modules.len() > MAX_SECTION_MODULES {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "section_modules",
                "at most 16 permitted module tokens per section",
            ));
        }
        for module in &self.modules {
            if !module_token_is_valid(module) {
                return Err(CommandError::new(
                    ErrorCode::InvalidArgument,
                    "section_modules",
                    "module token must be a lowercase namespace token",
                ));
            }
        }
        let mut sorted = self.modules.clone();
        sorted.sort();
        sorted.dedup();
        if sorted.len() != self.modules.len() {
            return Err(CommandError::new(
                ErrorCode::DuplicateId,
                "section_modules",
                "permitted module tokens must be unique",
            ));
        }
        Ok(())
    }
}

/// One optional project partition (F10 0.2 section 1): a stable ZoneId with label fields.
/// A zone never becomes an element's parent path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneRecord {
    /// Stable full UUIDv7 ZoneId (PC-ID-001).
    pub id: String,
    /// Human-readable code, a label only.
    pub code: String,
    /// Human-readable label.
    pub label: String,
}

impl ZoneRecord {
    /// Bounded admission (F12 section 9).
    pub fn validate(&self) -> Result<(), CommandError> {
        id(&self.id, "zone_id")?;
        code(&self.code, "zone_code")?;
        label(&self.label, "zone_label")
    }
}

/// The authored provenance of one resource (F10 0.2 DV-SCOPE-001/002, F12 PC-OWN-001): the
/// section that owns the authored content and its zone, or the unzoned partition when the
/// zone is absent. Unknown provenance gives no write permission; this record only states
/// what the store claims and every authorization re-checks it against the workspace scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoredScope {
    /// Owning full UUIDv7 SectionId, resolving in the project's section registry.
    pub section: String,
    /// Owning full UUIDv7 ZoneId; `None` is the unzoned partition of the section.
    pub zone: Option<String>,
}

impl AuthoredScope {
    /// Bounded admission: the zone exists only within a section.
    pub fn validate(&self) -> Result<(), CommandError> {
        id(&self.section, "scope_section")?;
        if let Some(zone) = &self.zone {
            id(zone, "scope_zone")?;
        }
        Ok(())
    }
}

/// The project descriptor's human metadata (F12 section 2): labels without authority. The
/// ProjectId itself is the existing root `bimProjectId` record and is not duplicated here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectMetadata {
    /// Project code label, 1 to 256 bytes.
    pub code: String,
    /// Project name label, 1 to 256 bytes.
    pub name: String,
    /// Owning organization label; absent stays unknown.
    pub organization: Option<String>,
}

impl ProjectMetadata {
    /// Bounded admission (F12 section 9).
    pub fn validate(&self) -> Result<(), CommandError> {
        label(&self.code, "project_code")?;
        label(&self.name, "project_name")?;
        if let Some(organization) = &self.organization {
            label(organization, "project_organization")?;
        }
        Ok(())
    }
}

/// The verdict of a host decision on one opening assignment (F10 0.2 section 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionVerdict {
    /// The host accepts the exact pinned proposal.
    Accepted,
    /// The host refuses the exact pinned proposal.
    Rejected,
}

impl DecisionVerdict {
    /// The exact stored token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
        }
    }
    /// Parses the exact stored token; nothing else is admitted.
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "accepted" => Some(Self::Accepted),
            "rejected" => Some(Self::Rejected),
            _ => None,
        }
    }
}

/// The closed refusal reason of a rejected decision (F10 0.1 section 5.4 list).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionReason {
    /// Refused because of placement.
    Position,
    /// Refused because of size.
    Size,
    /// Refused for structural reasons.
    Structural,
    /// Refused for another stated reason.
    Other,
}

impl DecisionReason {
    /// The exact stored token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Position => "position",
            Self::Size => "size",
            Self::Structural => "structural",
            Self::Other => "other",
        }
    }
    /// Parses the exact stored token; nothing else is admitted.
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "position" => Some(Self::Position),
            "size" => Some(Self::Size),
            "structural" => Some(Self::Structural),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

/// One recorded host decision on one opening assignment (F10 0.2 section 3): an immutable
/// evidence record that binds the verdict to the exact proposal bytes and the exact reviewed
/// host revision. Accepting a proposal never applies to a later revision of it, and the
/// decision never writes the proposer's store; whether an opening was actually authored in
/// the host model remains a separate fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpeningDecision {
    /// Stable full UUIDv7 of this decision record itself.
    pub id: String,
    /// Full UUIDv7 of the one shared assignment (proposal) identity this decision answers.
    pub proposal: String,
    /// Exact pin of the proposal content this decision binds (F12 PC-ID-002 pin rule).
    pub proposal_digest: ContentDigest,
    /// Full UUIDv7 of the reviewed host revision the decision was made against.
    pub host_revision: String,
    /// The host's verdict.
    pub verdict: DecisionVerdict,
    /// Closed refusal reason; present exactly when the verdict is `rejected`.
    pub reason: Option<DecisionReason>,
}

impl OpeningDecision {
    /// Bounded admission: a rejection carries a closed reason, an acceptance none.
    pub fn validate(&self) -> Result<(), CommandError> {
        id(&self.id, "decision_id")?;
        id(&self.proposal, "decision_proposal")?;
        id(&self.host_revision, "decision_host_revision")?;
        match (self.verdict, self.reason) {
            (DecisionVerdict::Rejected, Some(_)) | (DecisionVerdict::Accepted, None) => Ok(()),
            (DecisionVerdict::Rejected, None) => Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "decision_reason",
                "a rejection must carry a closed reason code",
            )),
            (DecisionVerdict::Accepted, Some(_)) => Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "decision_reason",
                "an acceptance never carries a refusal reason",
            )),
        }
    }
}

fn code(value: &str, field: &'static str) -> Result<(), CommandError> {
    if code_is_valid(value) {
        Ok(())
    } else {
        Err(CommandError::new(
            ErrorCode::InvalidArgument,
            field,
            "code must be 1 to 32 letters, digits, underscores or hyphens",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0199e7a0-0000-7000-8000-0000000000aa";

    fn section() -> SectionRecord {
        SectionRecord {
            id: ID.into(),
            code: "KMD".into(),
            label: "Detailing".into(),
            modules: vec!["steel".into()],
        }
    }

    #[test]
    fn bounded_records_are_admitted_and_drift_is_refused() {
        section().validate().unwrap();
        ZoneRecord {
            id: ID.into(),
            code: "K1".into(),
            label: "Zone K1".into(),
        }
        .validate()
        .unwrap();
        AuthoredScope {
            section: ID.into(),
            zone: None,
        }
        .validate()
        .unwrap();
        ProjectMetadata {
            code: "P-1".into(),
            name: "Bridge".into(),
            organization: None,
        }
        .validate()
        .unwrap();
        OpeningDecision {
            id: ID.into(),
            proposal: ID.into(),
            proposal_digest: ContentDigest::from_bytes([7; 32]),
            host_revision: ID.into(),
            verdict: DecisionVerdict::Rejected,
            reason: Some(DecisionReason::Size),
        }
        .validate()
        .unwrap();

        let long_code = "A".repeat(33);
        assert_eq!(
            SectionRecord {
                code: long_code,
                ..section()
            }
            .validate()
            .unwrap_err()
            .code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            ZoneRecord {
                label: String::new(),
                ..ZoneRecord {
                    id: ID.into(),
                    code: "K1".into(),
                    label: "x".into()
                }
            }
            .validate()
            .unwrap_err()
            .code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            AuthoredScope {
                zone: Some("nope".into()),
                ..AuthoredScope {
                    section: ID.into(),
                    zone: None
                }
            }
            .validate()
            .unwrap_err()
            .code,
            ErrorCode::InvalidId
        );
        assert_eq!(
            OpeningDecision {
                verdict: DecisionVerdict::Accepted,
                reason: Some(DecisionReason::Other),
                ..OpeningDecision {
                    id: ID.into(),
                    proposal: ID.into(),
                    proposal_digest: ContentDigest::from_bytes([7; 32]),
                    host_revision: ID.into(),
                    verdict: DecisionVerdict::Rejected,
                    reason: Some(DecisionReason::Size)
                }
            }
            .validate()
            .unwrap_err()
            .code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            SectionRecord {
                modules: vec!["steel".into(), "steel".into()],
                ..section()
            }
            .validate()
            .unwrap_err()
            .code,
            ErrorCode::DuplicateId
        );
        assert_eq!(
            SectionRecord {
                modules: vec!["Steel".into()],
                ..section()
            }
            .validate()
            .unwrap_err()
            .code,
            ErrorCode::InvalidArgument
        );
        assert!(!code_is_valid("_lead"));
        assert!(!code_is_valid(""));
        assert!(code_is_valid("K1_b-2"));
        assert_eq!(DecisionVerdict::parse("maybe"), None);
        assert_eq!(DecisionReason::parse("size"), Some(DecisionReason::Size));
    }
}
