// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic revision layers and opaque proof bytes, MPL-2.0.

//! C03.2 host: evidence is immutable revision content, never a floating sidecar or USD layer.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_host::compaction::{self, Keep, Request, Safeguard};
use beamcraft_host::ledger::RecoveryPolicy;
use beamcraft_host::revision::{
    CommitOutcome, CommitPoint, EvidencePin, ExpectedBase, Injected, LayerEntry, LayerRole,
    MAX_ENTRIES, MAX_OPERATION_EVIDENCE_BYTES, Manifest, NoFault, RevisionDraft, RevisionError,
    RevisionStore, Uuid7,
};
use beamcraft_host::storage::BlobStore;
use beamcraft_model::digest::{ContentDigest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "c03-evidence-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn store(&self) -> RevisionStore {
        RevisionStore::new(BlobStore::new(&self.0).unwrap())
    }
    fn blob(&self, digest: ContentDigest) -> PathBuf {
        let text = digest.to_string();
        self.0.join("blobs").join(&text[..2]).join(text)
    }
    fn pin(&self, bytes: &[u8]) -> EvidencePin {
        let (digest, _) = self.store().blobs().publish(bytes).unwrap();
        EvidencePin::new(digest, bytes.len() as u64).unwrap()
    }
    fn draft(&self, n: u64) -> RevisionDraft {
        let entries = [
            ("root.usda", LayerRole::Authored),
            ("authored.usda", LayerRole::Authored),
            ("derived.usda", LayerRole::Derived),
        ]
        .into_iter()
        .map(|(name, role)| {
            let bytes = format!("#usda 1.0\n# synthetic {name} {n}\n");
            self.store().blobs().publish(bytes.as_bytes()).unwrap();
            LayerEntry::from_bytes(name, role, bytes.as_bytes()).unwrap()
        })
        .collect();
        RevisionDraft {
            project: id(1),
            workspace: id(2),
            revision: id(100 + n),
            op: id(200 + n),
            workspace_generation: n,
            schema: digest(b"synthetic schema"),
            root_layer: "root.usda".into(),
            entries,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn id(n: u64) -> Uuid7 {
    Uuid7::parse(&format!("019a7bf0-0000-7000-8000-{n:012x}")).unwrap()
}
fn digest(bytes: &[u8]) -> ContentDigest {
    Sha256::digest(bytes).unwrap()
}

#[test]
fn evidence_is_pinned_reopened_and_not_exposed_as_a_layer() {
    let fx = Fixture::new();
    let bytes = b"synthetic operation evidence\0not USD\xff";
    let pin = fx.pin(bytes);
    let draft = fx.draft(1);
    let committed = fx
        .store()
        .commit_with_evidence(
            &draft,
            &ExpectedBase::Initial,
            Some(pin),
            None,
            &mut NoFault,
        )
        .unwrap();
    let encoded = committed.info.manifest().encode();
    assert!(encoded.starts_with(b"beamcraft-revision-manifest 2\n"));
    assert_eq!(Manifest::parse(&encoded).unwrap().encode(), encoded);
    let reopened = fx.store().open().unwrap();
    assert_eq!(
        reopened.manifest().content().operation_evidence(),
        Some(pin)
    );
    assert_eq!(reopened.operation_evidence().unwrap().unwrap(), bytes);
    assert_eq!(reopened.manifest().content().entries().len(), 3);
    assert_eq!(
        reopened
            .layer_bytes("operation-evidence")
            .unwrap_err()
            .code(),
        beamcraft_model::ErrorCode::NotFound
    );
}

#[test]
fn legacy_manifest_bytes_and_absent_evidence_remain_explicit() {
    let fx = Fixture::new();
    let draft = fx.draft(1);
    let committed = fx.store().commit(&draft, &ExpectedBase::Initial).unwrap();
    let bytes = committed.info.manifest().encode();
    assert!(bytes.starts_with(b"beamcraft-revision-manifest 1\n"));
    assert!(
        !String::from_utf8(bytes.clone())
            .unwrap()
            .contains("evidence ")
    );
    assert_eq!(Manifest::parse(&bytes).unwrap().encode(), bytes);
    assert_eq!(
        fx.store().open().unwrap().operation_evidence().unwrap(),
        None
    );
    assert_eq!(
        fx.store()
            .commit_with_evidence(&draft, &ExpectedBase::Initial, None, None, &mut NoFault)
            .unwrap()
            .outcome,
        CommitOutcome::Replayed
    );
}

#[test]
fn changing_or_dropping_the_pin_under_one_operation_is_reuse() {
    let fx = Fixture::new();
    let draft = fx.draft(1);
    let first = fx.pin(b"reads source A");
    let other = fx.pin(b"reads source B");
    let accepted = fx
        .store()
        .commit_with_evidence(
            &draft,
            &ExpectedBase::Initial,
            Some(first),
            None,
            &mut NoFault,
        )
        .unwrap();
    for pin in [None, Some(other)] {
        assert_eq!(
            fx.store()
                .commit_with_evidence(&draft, &ExpectedBase::Initial, pin, None, &mut NoFault)
                .unwrap_err(),
            RevisionError::OpIdReuse
        );
        assert_eq!(
            fx.store().head().unwrap().unwrap().manifest_digest(),
            accepted.info.manifest_digest()
        );
    }
    assert_eq!(
        fx.store()
            .commit_with_evidence(
                &draft,
                &ExpectedBase::Initial,
                Some(first),
                None,
                &mut NoFault
            )
            .unwrap()
            .outcome,
        CommitOutcome::Replayed
    );
}

#[test]
fn missing_wrong_sized_and_corrupt_evidence_never_publish_a_root() {
    for mode in ["missing", "size", "digest"] {
        let fx = Fixture::new();
        let draft = fx.draft(1);
        let good = fx.pin(b"original proof");
        let pin = match mode {
            "missing" => EvidencePin::new(digest(b"not published"), 13).unwrap(),
            "size" => EvidencePin::new(good.digest(), good.size() + 1).unwrap(),
            _ => {
                fs::write(fx.blob(good.digest()), b"tampered proof").unwrap();
                good
            }
        };
        assert!(
            fx.store()
                .commit_with_evidence(
                    &draft,
                    &ExpectedBase::Initial,
                    Some(pin),
                    None,
                    &mut NoFault
                )
                .is_err()
        );
        assert!(fx.store().head().unwrap().is_none(), "{mode}");
    }
}

#[test]
fn every_read_rechecks_evidence_in_current_and_historical_revisions() {
    let fx = Fixture::new();
    let pin = fx.pin(b"original proof");
    let first = fx.draft(1);
    fx.store()
        .commit_with_evidence(
            &first,
            &ExpectedBase::Initial,
            Some(pin),
            None,
            &mut NoFault,
        )
        .unwrap();
    let opened = fx.store().open().unwrap();
    fs::write(fx.blob(pin.digest()), b"modified proof").unwrap();
    assert_eq!(
        opened.operation_evidence().unwrap_err(),
        RevisionError::CorruptOperationEvidence
    );
    assert_eq!(
        fx.store().open().unwrap_err(),
        RevisionError::CorruptOperationEvidence
    );
    assert_eq!(
        fx.store().open_at(1).unwrap_err(),
        RevisionError::CorruptOperationEvidence
    );
    fs::write(fx.blob(pin.digest()), b"original proof").unwrap();
    fx.store()
        .commit(&fx.draft(2), &ExpectedBase::Revision(first.revision))
        .unwrap();
    fs::remove_file(fx.blob(pin.digest())).unwrap();
    assert_eq!(
        fx.store().open().unwrap().operation_evidence().unwrap(),
        None
    );
    assert_eq!(
        fx.store().open_at(1).unwrap_err(),
        RevisionError::MissingOperationEvidence
    );
}

#[test]
fn evidence_size_caps_and_noncanonical_version_forms_are_enforced() {
    let fx = Fixture::new();
    assert_eq!(
        EvidencePin::new(digest(b"oversized"), MAX_OPERATION_EVIDENCE_BYTES + 1).unwrap_err(),
        RevisionError::LimitExceeded
    );
    let bytes = vec![b'e'; MAX_OPERATION_EVIDENCE_BYTES as usize];
    let pin = fx.pin(&bytes);
    let done = fx
        .store()
        .commit_with_evidence(
            &fx.draft(1),
            &ExpectedBase::Initial,
            Some(pin),
            None,
            &mut NoFault,
        )
        .unwrap();
    assert_eq!(
        fx.store()
            .open()
            .unwrap()
            .operation_evidence()
            .unwrap()
            .unwrap()
            .len() as u64,
        MAX_OPERATION_EVIDENCE_BYTES
    );
    let encoded = String::from_utf8(done.info.manifest().encode()).unwrap();
    for bad in [
        encoded.replacen("manifest 2", "manifest 1", 1),
        encoded.replacen("manifest 2", "manifest 3", 1),
        encoded.replace(
            &format!(" {}\n", pin.size()),
            &format!(" 0{}\n", pin.size()),
        ),
        encoded
            .lines()
            .filter(|line| !line.starts_with("evidence "))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    ] {
        assert_eq!(
            Manifest::parse(bad.as_bytes()).unwrap_err(),
            RevisionError::MalformedManifest
        );
    }
}

#[test]
fn faulted_commit_keeps_the_pin_bound_through_retry_and_recovery() {
    for stop in [
        CommitPoint::Checked,
        CommitPoint::IntentRecorded,
        CommitPoint::BlobsVerified,
        CommitPoint::ManifestStored,
        CommitPoint::RecordStaged,
        CommitPoint::Switched,
        CommitPoint::Recorded,
    ] {
        let fx = Fixture::new();
        let pin = fx.pin(b"proof survives the root switch");
        let draft = fx.draft(1);
        let mut fault = |at| if at == stop { Err(Injected) } else { Ok(()) };
        let result = fx.store().commit_with_evidence(
            &draft,
            &ExpectedBase::Initial,
            Some(pin),
            None,
            &mut fault,
        );
        if matches!(stop, CommitPoint::Switched | CommitPoint::Recorded) {
            assert_eq!(
                fx.store()
                    .open()
                    .unwrap()
                    .operation_evidence()
                    .unwrap()
                    .unwrap(),
                b"proof survives the root switch"
            );
        } else {
            assert!(result.is_err());
            assert!(fx.store().head().unwrap().is_none());
        }
        fx.store()
            .commit_with_evidence(
                &draft,
                &ExpectedBase::Initial,
                Some(pin),
                None,
                &mut NoFault,
            )
            .unwrap();
        let recovery = fx.store().recover(RecoveryPolicy::quiescent()).unwrap();
        assert!(
            !recovery
                .unreferenced
                .unwrap()
                .digests
                .contains(&pin.digest())
        );
        assert_eq!(fx.store().head().unwrap().unwrap().sequence(), 1);
        assert_eq!(
            fx.store()
                .open()
                .unwrap()
                .operation_evidence()
                .unwrap()
                .unwrap(),
            b"proof survives the root switch"
        );
    }
}

#[test]
fn compaction_keeps_pinned_evidence_and_prunes_only_dropped_evidence() {
    let fx = Fixture::new();
    let old = fx.pin(b"old command evidence");
    let current = fx.pin(b"current command evidence");
    let first = fx.draft(1);
    fx.store()
        .commit_with_evidence(
            &first,
            &ExpectedBase::Initial,
            Some(old),
            None,
            &mut NoFault,
        )
        .unwrap();
    fx.store()
        .commit_with_evidence(
            &fx.draft(2),
            &ExpectedBase::Revision(first.revision),
            Some(current),
            None,
            &mut NoFault,
        )
        .unwrap();
    let request = Request {
        keep: Keep::Last(1),
        pins: &[],
        ledger_tail: 0,
        orphan_grace_secs: 0,
    };
    compaction::compact(
        &fx.store(),
        &request,
        true,
        Safeguard::NoBackup,
        &mut compaction::NoFault,
    )
    .unwrap();
    assert!(!fx.blob(old.digest()).exists());
    assert!(fx.blob(current.digest()).exists());
    assert_eq!(
        fx.store()
            .open()
            .unwrap()
            .operation_evidence()
            .unwrap()
            .unwrap(),
        b"current command evidence"
    );
}

#[test]
#[allow(clippy::panic)]
fn an_unresolved_intent_protects_its_evidence_before_any_root_exists() {
    let fx = Fixture::new();
    let pin = fx.pin(b"unresolved command evidence");
    let draft = fx.draft(1);
    let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut crash = |point| {
            if point == CommitPoint::IntentRecorded {
                panic!("synthetic crash after synced intent");
            }
            Ok(())
        };
        fx.store()
            .commit_with_evidence(&draft, &ExpectedBase::Initial, Some(pin), None, &mut crash)
    }));
    assert!(stopped.is_err());
    assert!(fx.store().head().unwrap().is_none());
    let report = fx.store().recover(RecoveryPolicy::live(3600)).unwrap();
    assert!(!report.unreferenced.unwrap().digests.contains(&pin.digest()));
    fx.store()
        .commit_with_evidence(
            &draft,
            &ExpectedBase::Initial,
            Some(pin),
            None,
            &mut NoFault,
        )
        .unwrap();
    assert_eq!(
        fx.store()
            .open()
            .unwrap()
            .operation_evidence()
            .unwrap()
            .unwrap(),
        b"unresolved command evidence"
    );
}

#[test]
fn the_full_layer_cap_and_one_evidence_blob_fit_one_durable_intent() {
    let fx = Fixture::new();
    let mut draft = fx.draft(1);
    for n in draft.entries.len()..MAX_ENTRIES {
        let bytes = format!("#usda 1.0\n# original extra layer {n}\n");
        fx.store().blobs().publish(bytes.as_bytes()).unwrap();
        draft.entries.push(
            LayerEntry::from_bytes(
                &format!("extra-{n}.usda"),
                LayerRole::Authored,
                bytes.as_bytes(),
            )
            .unwrap(),
        );
    }
    let pin = fx.pin(b"evidence beyond the 64 layer slots");
    fx.store()
        .commit_with_evidence(
            &draft,
            &ExpectedBase::Initial,
            Some(pin),
            None,
            &mut NoFault,
        )
        .unwrap();
    let opened = fx.store().open().unwrap();
    assert_eq!(opened.manifest().content().entries().len(), MAX_ENTRIES);
    assert_eq!(
        opened.operation_evidence().unwrap().unwrap(),
        b"evidence beyond the 64 layer slots"
    );
    assert_eq!(
        fx.store()
            .commit_with_evidence(
                &draft,
                &ExpectedBase::Initial,
                Some(pin),
                None,
                &mut NoFault
            )
            .unwrap()
            .outcome,
        CommitOutcome::Replayed
    );
}
