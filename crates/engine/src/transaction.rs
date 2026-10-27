// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Atomic command transactions: `op_id` idempotency and the typed result of one accepted command
//! (F04 sections 6 and 8).
//!
//! The decision here is pure. The durable record of what an `op_id` did belongs to the host's
//! outcome ledger (U07); the engine is told only what the host proved, as [`PriorOp`], and
//! decides between a new application, a replay (same content: return the recorded result,
//! change nothing) and `OP_ID_REUSE` (different content under an accepted `op_id`). Content is
//! compared through [`beamcraft_model::command::CreatePlate::content_fingerprint`], a typed-argument digest; a
//! canonical-byte comparison (F04 section 3) is planned and replaces it without changing this API.

use crate::command::{CreatePlateOutcome, MaterialLookup, create_plate};
use beamcraft_model::command::{CommandError, CreatePlate};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId};

/// What the host proved about an operation id before this call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorOp {
    /// The host has no record of acceptance (never seen, or every attempt was closed as not
    /// accepted).
    Unseen,
    /// The operation was accepted; `fingerprint` is the content fingerprint of the accepted
    /// request as the host can reconstruct it.
    Accepted {
        /// Fingerprint of the accepted request.
        fingerprint: ContentDigest,
    },
}

/// One accepted command: the operation, its derived results and the footprint it declared.
#[derive(Debug, Clone, PartialEq)]
pub struct Transaction {
    /// The operation id bound to this application.
    pub op_id: OpId,
    /// Authored state, body, quantities, footprint and content fingerprint.
    pub outcome: CreatePlateOutcome,
}

/// The decision of [`admit_create_plate`].
#[derive(Debug, Clone, PartialEq)]
pub enum Admission {
    /// A new operation: apply it. Nothing is changed until the host commits.
    Apply(Box<Transaction>),
    /// The same operation with identical content was already accepted: return the recorded
    /// result and change nothing (no new generation, no new revision).
    Replay,
}

/// Admits one `CreatePlate` for application.
///
/// An already accepted `op_id` is decided first (F04 section 8 order): identical content is
/// [`Admission::Replay`], different content `OP_ID_REUSE`. A new operation is validated and
/// evaluated by [`create_plate`]. Any error leaves authored state, derived state and generation
/// unchanged because nothing was applied.
pub fn admit_create_plate(
    op_id: OpId,
    command: &CreatePlate,
    prior: PriorOp,
    existing: &dyn Fn(&ElementId) -> bool,
    materials: MaterialLookup<'_>,
) -> Result<Admission, CommandError> {
    match prior {
        PriorOp::Accepted { fingerprint } => {
            if command.content_fingerprint()? == fingerprint {
                Ok(Admission::Replay)
            } else {
                Err(CommandError::new(
                    ErrorCode::OpIdReuse,
                    "op_id",
                    "operation id was accepted with different content",
                ))
            }
        }
        PriorOp::Unseen => Ok(Admission::Apply(Box::new(Transaction {
            op_id,
            outcome: create_plate(command, existing, materials)?,
        }))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::no_materials;
    use beamcraft_model::command::Placement;

    // Shadows the glob import: these tests have no material definitions.
    fn admit_create_plate(
        op_id: OpId,
        command: &CreatePlate,
        prior: PriorOp,
        existing: &dyn Fn(&ElementId) -> bool,
    ) -> Result<Admission, CommandError> {
        super::admit_create_plate(op_id, command, prior, existing, &no_materials)
    }

    fn command(width: f64) -> CreatePlate {
        CreatePlate {
            new_id: ElementId::parse("019a7bf0-0000-7000-8000-000000000005").unwrap(),
            name: "Synthetic plate".into(),
            class: 0,
            contour_points_mm: vec![
                [0.0, 0.0, 0.0],
                [width, 0.0, 0.0],
                [width, 500.0, 0.0],
                [0.0, 500.0, 0.0],
            ],
            thickness_mm: 10.0,
            thickness_side: None,
            corners: Vec::new(),
            placement: Placement::IDENTITY,
            material_digest: None,
            holes_mm: Vec::new(),
        }
    }

    fn op() -> OpId {
        OpId::parse("019a7bf0-0000-7000-8000-000000000004").unwrap()
    }

    fn none(_: &ElementId) -> bool {
        false
    }

    #[test]
    fn an_unseen_operation_is_applied_with_its_footprint_and_fingerprint() {
        let Admission::Apply(t) =
            admit_create_plate(op(), &command(1000.0), PriorOp::Unseen, &none).unwrap()
        else {
            panic!("expected apply");
        };
        assert_eq!(t.op_id, op());
        assert_eq!(
            t.outcome.fingerprint,
            command(1000.0).content_fingerprint().unwrap()
        );
        assert!(!t.outcome.footprint.writes().is_empty());
    }

    #[test]
    fn identical_content_replays_and_changed_content_is_reuse() {
        let accepted = PriorOp::Accepted {
            fingerprint: command(1000.0).content_fingerprint().unwrap(),
        };
        // F04-POS-04: the recorded result is returned, nothing is applied.
        assert_eq!(
            admit_create_plate(op(), &command(1000.0), accepted, &none).unwrap(),
            Admission::Replay
        );
        // F04-NEG-10: the width changed under the same op id.
        let error = admit_create_plate(op(), &command(750.0), accepted, &none).unwrap_err();
        assert_eq!(error.code, ErrorCode::OpIdReuse);
    }

    #[test]
    fn a_rejection_applies_nothing() {
        let mut bad = command(1000.0);
        bad.thickness_mm = 0.0;
        assert_eq!(
            admit_create_plate(op(), &bad, PriorOp::Unseen, &none)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        let taken = |id: &ElementId| *id == command(1000.0).new_id;
        assert_eq!(
            admit_create_plate(op(), &command(1000.0), PriorOp::Unseen, &taken)
                .unwrap_err()
                .code,
            ErrorCode::DuplicateId
        );
    }
}
