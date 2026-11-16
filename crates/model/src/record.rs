// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Q03 recorded inputs: joint intent and gap requirements as authored model data
//! (contract sections 4 and 5 of `docs/contracts/q03-model-checks-draft-v0.1.md`).
//!
//! A record is an authored element of kind `record` with no engineering body, no material
//! and the identity placement. Its payload is either the declared joint intent — the scope
//! of the joint and the connections the owner declares must join its parts — or the gap
//! requirement of one declared weld connection: the permitted gap range with the tolerance
//! on the comparison. Both are check inputs for `check-model`: the engine never infers
//! them from geometry or from a weld, and only explicit commands create them. A record
//! never blocks export or publication, and deleting it is an explicit `delete-element`.

use crate::command::{CommandError, label_is_valid};
use crate::digest::{ContentDigest, Sha256};
use crate::error::ErrorCode;
use crate::id::ElementId;
use crate::limits::DIMENSION_CAP_MM;

/// The most records of one project.
pub const MAX_RECORDS: usize = 4096;

/// The most elements of one declared joint scope.
pub const MAX_SCOPE: usize = 256;

/// The most declared connections of one joint intent.
pub const MAX_CONNECTIONS: usize = 256;

/// The declared kind of one connection (contract section 5). Bolted kinds are refused in
/// v0.1 and arrive with the fastener slices, so no variant exists for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionKind {
    /// The parts must be joined by a weld element.
    Weld,
    /// The parts must be in physical contact within the product tolerance.
    Contact,
}

impl ConnectionKind {
    /// The stored token of the kind.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Weld => "weld",
            Self::Contact => "contact",
        }
    }

    /// The kind of a stored token, and nothing else.
    pub fn of_token(token: &str) -> Option<Self> {
        match token {
            "weld" => Some(Self::Weld),
            "contact" => Some(Self::Contact),
            _ => None,
        }
    }
}

/// One declared connection of a joint intent (contract section 5): two distinct part
/// elements and what the owner declared must join them. Connection notes are not stored
/// in v0.1; the contract marks the note optional.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredConnection {
    /// One side of the declared connection.
    pub a: ElementId,
    /// The other side, distinct from `a`.
    pub b: ElementId,
    /// What the owner declared must join the parts.
    pub kind: ConnectionKind,
}

/// The declared joint intent (contract section 5): the elements that form the joint and
/// the connections the owner declares within it. Intent answers only "what did the owner
/// declare must be connected here"; it never infers a connection from contact or contact
/// from a weld.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JointIntent {
    /// The element ids that form the joint, distinct.
    pub scope: Vec<ElementId>,
    /// The declared connections between the joint's parts.
    pub connections: Vec<DeclaredConnection>,
}

/// What the owner recorded a corner-cut requirement for (contract section 4.1). The kind
/// names the purpose of the cut; no supported rule distinguishes them in v0.1, so the
/// kind is provenance carried with the requirement, not a rule selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutPurpose {
    /// Space for the joint at the corner (the sample's case).
    Clearance,
    /// Room to reach the joint beyond an obstruction.
    Access,
}

impl CutPurpose {
    /// The stored token of the purpose.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Clearance => "clearance",
            Self::Access => "access",
        }
    }

    /// The purpose of a stored token, and nothing else.
    pub fn of_token(token: &str) -> Option<Self> {
        match token {
            "clearance" => Some(Self::Clearance),
            "access" => Some(Self::Access),
            _ => None,
        }
    }
}

/// A recorded standalone upper bound on the corner cut (contract section 4.1): the
/// owner's own "no more than X" limit, which replaces the acceptance band's upper edge
/// (the recorded size) when present. No normative upper bound exists, so the bound is
/// never invented — it is only ever this recorded one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UpperBound {
    /// The standalone maximum cut size in millimetres.
    pub max_mm: f64,
    /// The recorded tolerance on that maximum (contract section 7.3).
    pub tolerance_mm: f64,
}

/// The recorded corner-cut requirement of one rib corner (contract section 4.1): the rib
/// element, the two neighbour elements whose surfaces sandwich the corner, the required
/// cut size with its tolerance, an optional standalone upper bound, and the recorded
/// purpose. The corner itself is never stored: on every run the check re-derives the
/// rib's corner adjacent to both neighbours from the final evaluated geometry, and an
/// edit that leaves it ambiguous reports `target_unresolved` instead of retargeting
/// (contract section 4.3). The basis is the owner in v0.1 — no normative corner-cut rule
/// exists — so no basis field is stored, and the record's label carries the owner's note.
#[derive(Debug, Clone, PartialEq)]
pub struct CornerCutRequirement {
    /// The rib element carrying the corner.
    pub target: ElementId,
    /// The two neighbours whose surfaces sandwich the corner, in any order.
    pub neighbours: [ElementId; 2],
    /// The required cut size in millimetres (the sample: 10.0). The acceptance band's
    /// upper edge is this size plus the tolerance, so the recorded size is itself the
    /// upper bound a `CORNER_OVERCUT` verdict uses unless a standalone bound is present.
    pub required_mm: f64,
    /// The recorded tolerance on the comparison (contract section 7.3).
    pub tolerance_mm: f64,
    /// The recorded standalone maximum, when the owner states one.
    pub upper_bound: Option<UpperBound>,
    /// The recorded purpose of the cut.
    pub purpose: CutPurpose,
}

/// The recorded gap requirement of one declared weld connection (contract section 4.1):
/// the permitted local gap range with the tolerance on the comparison. The basis is the
/// owner in v0.1 — no normative permitted-gap rule is supported — so no basis field is
/// stored. `permitted_min_mm` must not exceed `permitted_max_mm`, and two declared
/// connections of one intent never share an unordered pair, so the requirement's pair
/// names at most one declared connection.
#[derive(Debug, Clone, PartialEq)]
pub struct GapRequirement {
    /// One side of the declared weld connection the requirement judges.
    pub a: ElementId,
    /// The other side, distinct from `a`.
    pub b: ElementId,
    /// The permitted minimum local gap in millimetres.
    pub permitted_min_mm: f64,
    /// The permitted maximum local gap in millimetres.
    pub permitted_max_mm: f64,
    /// The recorded tolerance on the comparison (contract section 7.3).
    pub tolerance_mm: f64,
}

/// What one record stores.
#[derive(Debug, Clone, PartialEq)]
pub enum RecordPayload {
    /// The declared joint intent.
    JointIntent(JointIntent),
    /// The gap requirement of one declared weld connection.
    GapRequirement(GapRequirement),
    /// The corner-cut requirement of one rib corner.
    CornerCut(CornerCutRequirement),
}

/// One recorded input: an authored element of kind `record`.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    /// Stable element identity.
    pub id: ElementId,
    /// Label, 1 to 256 UTF-8 bytes without control characters. Not an identity.
    pub name: String,
    /// What the record stores.
    pub payload: RecordPayload,
}

fn admit_dimension(
    value: f64,
    field: &'static str,
    what: &'static str,
) -> Result<(), CommandError> {
    if !value.is_finite() || value.abs() > DIMENSION_CAP_MM {
        return Err(CommandError::new(ErrorCode::InvalidArgument, field, what));
    }
    Ok(())
}

fn admit_corner_cut(corner: &CornerCutRequirement) -> Result<(), CommandError> {
    if corner.target == corner.neighbours[0]
        || corner.target == corner.neighbours[1]
        || corner.neighbours[0] == corner.neighbours[1]
    {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "target",
            "a corner-cut requirement names three distinct elements: the rib and two \
             neighbours",
        ));
    }
    admit_dimension(
        corner.required_mm,
        "required_mm",
        "must be a bounded millimetre value",
    )?;
    admit_dimension(
        corner.tolerance_mm,
        "tolerance_mm",
        "must be a bounded millimetre value",
    )?;
    if corner.required_mm <= 0.0 {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "required_mm",
            "a required corner cut is a positive size",
        ));
    }
    if corner.tolerance_mm < 0.0 {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "tolerance_mm",
            "a tolerance is never negative",
        ));
    }
    match &corner.upper_bound {
        None => {}
        Some(bound) => {
            admit_dimension(
                bound.max_mm,
                "upper_bound_mm",
                "must be a bounded millimetre value",
            )?;
            admit_dimension(
                bound.tolerance_mm,
                "upper_bound_tolerance_mm",
                "must be a bounded millimetre value",
            )?;
            if bound.max_mm <= 0.0 {
                return Err(CommandError::new(
                    ErrorCode::InvalidArgument,
                    "upper_bound_mm",
                    "a recorded maximum is a positive size",
                ));
            }
            if bound.tolerance_mm < 0.0 {
                return Err(CommandError::new(
                    ErrorCode::InvalidArgument,
                    "upper_bound_tolerance_mm",
                    "a tolerance is never negative",
                ));
            }
            if corner.required_mm - corner.tolerance_mm >= bound.max_mm + bound.tolerance_mm {
                return Err(CommandError::new(
                    ErrorCode::InvalidArgument,
                    "upper_bound_mm",
                    "the recorded maximum must leave the acceptance band non-empty",
                ));
            }
        }
    }
    Ok(())
}

fn admit_intent(intent: &JointIntent) -> Result<(), CommandError> {
    if intent.scope.len() < 2 || intent.scope.len() > MAX_SCOPE {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "scope",
            "a joint intent declares two to 256 elements",
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for id in &intent.scope {
        if !seen.insert(*id) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "scope",
                "the declared scope names each element once",
            ));
        }
    }
    if intent.connections.len() > MAX_CONNECTIONS {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "connections",
            "a joint intent declares at most 256 connections",
        ));
    }
    let mut pairs = std::collections::BTreeSet::new();
    for connection in &intent.connections {
        if connection.a == connection.b {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "connections",
                "a declared connection joins two distinct elements",
            ));
        }
        let forward = pairs.insert((connection.a, connection.b));
        let reverse = pairs.insert((connection.b, connection.a));
        if !forward || !reverse {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "connections",
                "each unordered pair is declared at most once",
            ));
        }
    }
    Ok(())
}

fn admit_gap(gap: &GapRequirement) -> Result<(), CommandError> {
    if gap.a == gap.b {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "connection",
            "a gap requirement judges two distinct elements",
        ));
    }
    admit_dimension(
        gap.permitted_min_mm,
        "permitted_min_mm",
        "must be a bounded millimetre value",
    )?;
    admit_dimension(
        gap.permitted_max_mm,
        "permitted_max_mm",
        "must be a bounded millimetre value",
    )?;
    admit_dimension(
        gap.tolerance_mm,
        "tolerance_mm",
        "must be a bounded millimetre value",
    )?;
    if gap.permitted_min_mm < 0.0 {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "permitted_min_mm",
            "a permitted gap is never negative",
        ));
    }
    if gap.permitted_min_mm > gap.permitted_max_mm {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "permitted_min_mm",
            "the permitted minimum must not exceed the maximum",
        ));
    }
    if gap.tolerance_mm < 0.0 {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "tolerance_mm",
            "a tolerance is never negative",
        ));
    }
    Ok(())
}

impl Record {
    /// Admits the label and the payload, in that order.
    pub fn validate(&self) -> Result<(), CommandError> {
        if !label_is_valid(&self.name) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "name",
                "label must be 1 to 256 bytes without control characters",
            )
            .with_rule("DV-BASE-003"));
        }
        match &self.payload {
            RecordPayload::JointIntent(intent) => admit_intent(intent),
            RecordPayload::GapRequirement(gap) => admit_gap(gap),
            RecordPayload::CornerCut(corner) => admit_corner_cut(corner),
        }
    }

    /// The stored token of the payload's kind, the `steel:record:type` value.
    pub const fn payload_token(&self) -> &'static str {
        match self.payload {
            RecordPayload::JointIntent(_) => "joint_intent",
            RecordPayload::GapRequirement(_) => "gap_requirement",
            RecordPayload::CornerCut(_) => "corner_cut",
        }
    }
}

/// Q03 recorded-input creation request.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateRecord {
    /// The complete record to create.
    pub record: Record,
}

impl CreateRecord {
    /// Admits all model arguments before consulting project state.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        self.record.validate()
    }

    /// Versioned, length-prefixed fingerprint of all authored input, as [`crate::command`]
    /// commands do; it is not the F04 canonical form.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"create-record:v1:");
        bytes.extend_from_slice(&self.record.id.to_string().into_bytes());
        bytes.extend_from_slice(self.record.name.as_bytes());
        match &self.record.payload {
            RecordPayload::JointIntent(intent) => {
                bytes.extend_from_slice(b"intent:");
                for id in &intent.scope {
                    bytes.extend_from_slice(&id.to_string().into_bytes());
                    bytes.push(b',');
                }
                bytes.push(b';');
                for connection in &intent.connections {
                    bytes.extend_from_slice(&connection.a.to_string().into_bytes());
                    bytes.push(b'|');
                    bytes.extend_from_slice(&connection.b.to_string().into_bytes());
                    bytes.push(b'|');
                    bytes.extend_from_slice(connection.kind.token().as_bytes());
                    bytes.push(b',');
                }
            }
            RecordPayload::GapRequirement(gap) => {
                bytes.extend_from_slice(b"gap:");
                bytes.extend_from_slice(&gap.a.to_string().into_bytes());
                bytes.push(b'|');
                bytes.extend_from_slice(&gap.b.to_string().into_bytes());
                bytes.push(b'|');
                bytes.extend_from_slice(&gap.permitted_min_mm.to_le_bytes());
                bytes.extend_from_slice(&gap.permitted_max_mm.to_le_bytes());
                bytes.extend_from_slice(&gap.tolerance_mm.to_le_bytes());
            }
            RecordPayload::CornerCut(corner) => {
                bytes.extend_from_slice(b"corner:");
                bytes.extend_from_slice(&corner.target.to_string().into_bytes());
                for neighbour in corner.neighbours {
                    bytes.push(b'|');
                    bytes.extend_from_slice(&neighbour.to_string().into_bytes());
                }
                bytes.extend_from_slice(&corner.required_mm.to_le_bytes());
                bytes.extend_from_slice(&corner.tolerance_mm.to_le_bytes());
                match &corner.upper_bound {
                    None => bytes.push(b'-'),
                    Some(bound) => {
                        bytes.push(b'+');
                        bytes.extend_from_slice(&bound.max_mm.to_le_bytes());
                        bytes.extend_from_slice(&bound.tolerance_mm.to_le_bytes());
                    }
                }
                bytes.extend_from_slice(corner.purpose.token().as_bytes());
            }
        }
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(
                ErrorCode::InvalidArgument,
                "fingerprint",
                "the content fingerprint could not be computed",
            )
        })
    }
}
