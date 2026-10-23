// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic requests and malicious bytes, MPL-2.0.

//! C03.8 receipt codec. These tests prove bounded encoding and binding, not runtime rebase,
//! source provenance or recovery of a materialized model.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_cli::command_evidence::CapturedFootprint;
use beamcraft_cli::workspace::{
    WorkspaceMeta,
    rebase_receipt::{RebaseReceipt, RevisionPin},
};
use beamcraft_engine::footprint::{Entry, Footprint, Group, Target};
use beamcraft_host::revision::{MAX_OPERATION_EVIDENCE_BYTES, MAX_REVISIONS, Uuid7};
use beamcraft_model::changeset::{MAX_OPERATIONS, Operation};
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::id::{ElementId, OpId};
use beamcraft_model::limits::MAX_FOOTPRINT_ENTRIES;

fn id(n: usize) -> Uuid7 {
    Uuid7::parse(&format!("019a7bf0-0000-7000-8000-{n:012x}")).unwrap()
}
fn op(n: usize) -> OpId {
    OpId::parse(id(n).as_str()).unwrap()
}
fn digest(n: usize) -> ContentDigest {
    Sha256::digest(&n.to_le_bytes()).unwrap()
}
fn sample() -> RebaseReceipt {
    RebaseReceipt {
        op_id: op(99),
        project: id(10),
        schema: digest(20),
        old_meta: WorkspaceMeta {
            workspace_id: id(11),
            user_id: id(12),
            base_revision: id(1),
            base_manifest: digest(1),
            synced_sequence: 1,
            rebase_sequence: 0,
        },
        old_base_sequence: 5,
        draft: RevisionPin {
            revision: id(3),
            manifest: digest(3),
            sequence: 2,
        },
        accepted: RevisionPin {
            revision: id(2),
            manifest: digest(2),
            sequence: 6,
        },
        operations: vec![Operation {
            op_id: op(42),
            result: digest(3),
        }],
        footprint: Footprint::for_update(
            ElementId::parse(id(30).as_str()).unwrap(),
            true,
            false,
            false,
            false,
            None,
        )
        .unwrap(),
    }
}
fn blocks(bytes: &[u8]) -> (&str, &str) {
    let text = std::str::from_utf8(bytes).unwrap();
    let (_, rest) = text.split_once('\n').unwrap();
    let (count, rest) = rest.split_once('\n').unwrap();
    let n: usize = count.strip_prefix("request ").unwrap().parse().unwrap();
    let (request, rest) = rest.split_at(n);
    let (_, proof) = rest.split_once('\n').unwrap();
    (request, proof)
}
fn wrap(request: &str, proof: &str) -> Vec<u8> {
    format!(
        "beamcraft-workspace-rebase 1\nrequest {}\n{request}footprint {}\n{proof}",
        request.len(),
        proof.len()
    )
    .into_bytes()
}

#[test]
fn canonical_receipts_bind_all_input_coordinates_without_advancing_the_local_prefix() {
    let receipt = sample();
    let bytes = receipt.encode().unwrap();
    let decoded = RebaseReceipt::decode(&bytes).unwrap();
    assert_eq!(decoded, receipt);
    assert_eq!(decoded.old_meta.synced_sequence, 1);
    let (request, proof) = blocks(&bytes);
    let captured = CapturedFootprint::decode(proof.as_bytes()).unwrap();
    assert_eq!(captured.op_id, receipt.op_id);
    assert_eq!(captured.fingerprint, receipt.fingerprint().unwrap());
    assert_eq!(
        captured.fingerprint,
        Sha256::digest(request.as_bytes()).unwrap()
    );
    assert_eq!(captured.footprint, receipt.footprint);
    for variant in 0..7 {
        let mut changed = receipt.clone();
        match variant {
            0 => changed.old_meta.user_id = id(100),
            1 => changed.project = id(100),
            2 => changed.schema = digest(100),
            3 => changed.accepted.manifest = digest(100),
            4 => changed.draft.revision = id(100),
            5 => changed.old_base_sequence = 4,
            _ => changed.old_meta.rebase_sequence = 2,
        }
        assert_ne!(
            changed.fingerprint().unwrap(),
            receipt.fingerprint().unwrap()
        );
        assert_eq!(
            RebaseReceipt::decode(&changed.encode().unwrap()).unwrap(),
            changed
        );
    }
}

#[test]
fn a_footprint_for_another_request_or_operation_cannot_be_substituted() {
    let receipt = sample();
    let bytes = receipt.encode().unwrap();
    let (request, proof) = blocks(&bytes);
    let captured = CapturedFootprint::decode(proof.as_bytes()).unwrap();
    for wrong in [
        CapturedFootprint {
            op_id: op(100),
            ..captured.clone()
        },
        CapturedFootprint {
            fingerprint: digest(100),
            ..captured
        },
    ] {
        let wrong = wrong.encode().unwrap();
        assert!(
            RebaseReceipt::decode(&wrap(request, std::str::from_utf8(&wrong).unwrap())).is_err()
        );
    }
    let forged = request.replace(
        &format!("accepted {}", receipt.accepted.revision),
        &format!("accepted {}", id(101)),
    );
    assert!(
        RebaseReceipt::decode(&wrap(&forged, proof)).is_err(),
        "a new accepted pin requires a new request fingerprint"
    );
}

#[test]
fn truncated_noncanonical_or_unsupported_bytes_never_produce_a_partial_receipt() {
    let bytes = sample().encode().unwrap();
    for n in 0..bytes.len() {
        assert!(RebaseReceipt::decode(&bytes[..n]).is_err(), "prefix {n}");
    }
    let text = std::str::from_utf8(&bytes).unwrap();
    for malformed in [
        text.replacen("rebase 1", "rebase 2", 1),
        format!("{text}\n"),
        text.replace('\n', "\r\n"),
        text.replacen("request ", "request 0", 1),
        text.replacen("footprint ", "footprint +", 1),
        text.replacen("prefix 1", "prefix 01", 1),
        text.replacen("operations 1", "operations 4097", 1),
        text.replacen("operations 1", "operations 18446744073709551615", 1),
        text.replacen("request ", "unknown ", 1),
    ] {
        assert!(RebaseReceipt::decode(malformed.as_bytes()).is_err());
    }
    assert!(RebaseReceipt::decode(&vec![0xff; MAX_OPERATION_EVIDENCE_BYTES as usize + 1]).is_err());
    let huge = b"beamcraft-workspace-rebase 1\nrequest 18446744073709551615\n";
    assert!(RebaseReceipt::decode(huge).is_err());
}

#[test]
fn inconsistent_sequences_id_reuse_and_incomplete_source_intervals_are_refused() {
    for variant in 0..12 {
        let mut receipt = sample();
        match variant {
            0 => receipt.old_meta.synced_sequence = 0,
            1 => receipt.draft.sequence = 1,
            2 => receipt.draft.sequence = MAX_REVISIONS,
            3 => receipt.old_base_sequence = receipt.accepted.sequence,
            4 => receipt.accepted.sequence = MAX_REVISIONS + 1,
            5 => receipt.accepted.revision = receipt.old_meta.base_revision.clone(),
            6 => receipt.operations.clear(),
            7 => receipt.operations[0].result = digest(100),
            8 => receipt.operations[0].op_id = receipt.op_id,
            9 => {
                receipt.operations.push(receipt.operations[0]);
                receipt.draft.sequence += 1;
            }
            10 => receipt.op_id = OpId::parse(receipt.draft.revision.as_str()).unwrap(),
            _ => receipt.footprint = Footprint::default(),
        }
        assert!(receipt.encode().is_err(), "variant {variant}");
    }
}

#[test]
fn independent_operation_and_dependency_caps_are_checked_before_expansion() {
    let mut receipt = sample();
    receipt.op_id = op(100_000);
    receipt.operations = (0..MAX_OPERATIONS)
        .map(|n| Operation {
            op_id: op(1000 + n),
            result: digest(1000 + n),
        })
        .collect();
    receipt.draft.sequence = 1 + MAX_OPERATIONS as u64;
    receipt.draft.manifest = receipt.operations.last().unwrap().result;
    let reads: Vec<_> = (0..MAX_FOOTPRINT_ENTRIES)
        .map(|n| Entry {
            target: Target::Element(ElementId::parse(id(n + 1).as_str()).unwrap()),
            group: Group::Lifecycle,
        })
        .collect();
    receipt.footprint = Footprint::from_entries(&reads, &[]).unwrap();
    let bytes = receipt.encode().unwrap();
    assert!(bytes.len() as u64 <= MAX_OPERATION_EVIDENCE_BYTES);
    assert_eq!(RebaseReceipt::decode(&bytes).unwrap(), receipt);
    receipt.operations.push(Operation {
        op_id: op(9000),
        result: digest(9000),
    });
    receipt.draft.sequence += 1;
    receipt.draft.manifest = digest(9000);
    assert!(receipt.encode().is_err());
}
