// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original generated synthetic fixtures, MPL-2.0. Expectations follow F04 section 8
// and the U07 plan row; no third-party layer or asset is used.

//! U07 end to end: an interrupted commit is produced through the real host commit path and its
//! hook (the commit "dies" at a chosen point, so nothing after it runs), then the real `beamcraft`
//! binary answers `inspect-revision-outcome`, runs `recover-revision` and a retry of `commit-revision`
//! in fresh processes. The root always opens as one complete revision.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use beamcraft_host::{
    ledger::{CommittedRecord, Record},
    revision::{
        CommitPoint, ExpectedBase, Injected, LayerEntry, LayerRole, RevisionDraft, RevisionStore,
        Uuid7,
    },
    storage::BlobStore,
};
use beamcraft_json::{Json, Limits};
use beamcraft_model::digest::{ContentDigest, Sha256};

const PROJECT: &str = "0199e7a0-0000-7000-8000-000000000001";
const WORKSPACE: &str = "0199e7a0-0000-7000-8000-000000000002";
const REV1: &str = "0199e7a0-0000-7000-8000-0000000000a1";
const REV2: &str = "0199e7a0-0000-7000-8000-0000000000a2";
const OP1: &str = "0199e7a0-0000-7000-8000-0000000000b1";
const OP2: &str = "0199e7a0-0000-7000-8000-0000000000b2";
const OTHER: &str = "0199e7a0-0000-7000-8000-0000000000b9";

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("u07-tests");
        fs::create_dir_all(&base).unwrap();
        let root = base.join(format!(
            "e2e-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("store")).unwrap();
        fs::create_dir(root.join("files")).unwrap();
        Self(root)
    }
    fn store(&self) -> PathBuf {
        self.0.join("store")
    }
    fn revisions(&self) -> RevisionStore {
        RevisionStore::new(BlobStore::new(self.store()).unwrap())
    }
    fn ledger(&self) -> PathBuf {
        self.store().join("ledger").join("outcomes.log")
    }
    fn run(&self, args: &[&str]) -> (i32, Json) {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .output()
            .unwrap();
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
        (output.status.code().unwrap(), body)
    }
    fn store_arg(&self) -> String {
        self.store().to_str().unwrap().to_owned()
    }
    fn outcome(&self, op: &str) -> (i32, Json) {
        self.run(&[
            "inspect-revision-outcome",
            "--store",
            &self.store_arg(),
            "--op-id",
            op,
        ])
    }
    fn recover(&self, min_age: &str) -> (i32, Json) {
        self.run(&[
            "recover-revision",
            "--store",
            &self.store_arg(),
            "--min-age-secs",
            min_age,
        ])
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.0.is_absolute() && self.0.file_name().is_some());
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn text(json: &Json, key: &str) -> String {
    json.get(key).unwrap().string().unwrap().to_owned()
}
fn ids(json: &Json, key: &str) -> Vec<String> {
    json.get(key)
        .unwrap()
        .array()
        .unwrap()
        .iter()
        .map(|v| v.string().unwrap().to_owned())
        .collect()
}
fn schema() -> ContentDigest {
    Sha256::digest(b"synthetic schema identity").unwrap()
}

/// Four synthetic layers of generation `n` as `(name, role, bytes)`.
fn layer_set(n: u32) -> Vec<(&'static str, LayerRole, Vec<u8>)> {
    [
        ("root.usda", LayerRole::Authored),
        ("authored.usda", LayerRole::Authored),
        ("derived.usda", LayerRole::Derived),
        ("weak.usda", LayerRole::External),
    ]
    .into_iter()
    .map(|(name, role)| {
        (
            name,
            role,
            format!("#usda 1.0\n# synthetic {name} generation {n}\n").into_bytes(),
        )
    })
    .collect()
}

/// Stores the layers with the real `put-blob` command.
fn blob_put(fx: &Fixture, n: u32) {
    for (name, _, bytes) in layer_set(n) {
        let file = fx.0.join("files").join(format!("{name}.{n}"));
        fs::write(&file, bytes).unwrap();
        let (exit, _) = fx.run(&[
            "put-blob",
            "--store",
            &fx.store_arg(),
            "--file",
            file.to_str().unwrap(),
        ]);
        assert_eq!(exit, 0);
    }
}

fn layers_arg(n: u32) -> String {
    layer_set(n)
        .iter()
        .map(|(name, role, bytes)| {
            format!(
                "{}:{name}:{}",
                role.as_str(),
                Sha256::digest(bytes).unwrap()
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// `commit-revision` through the real binary.
fn commit(fx: &Fixture, n: u32, revision: &str, op: &str, base: &str) -> (i32, Json) {
    let generation = (40 + n).to_string();
    fx.run(&[
        "commit-revision",
        "--store",
        &fx.store_arg(),
        "--project-id",
        PROJECT,
        "--workspace-id",
        WORKSPACE,
        "--revision-id",
        revision,
        "--op-id",
        op,
        "--generation",
        &generation,
        "--base",
        base,
        "--schema",
        &schema().to_string(),
        "--root-layer",
        "root.usda",
        "--layers",
        &layers_arg(n),
    ])
}

/// The same draft the binary builds, for the in-process crash simulation.
fn draft(n: u32, revision: &str, op: &str) -> RevisionDraft {
    RevisionDraft {
        project: Uuid7::parse(PROJECT).unwrap(),
        workspace: Uuid7::parse(WORKSPACE).unwrap(),
        revision: Uuid7::parse(revision).unwrap(),
        op: Uuid7::parse(op).unwrap(),
        workspace_generation: 40 + u64::from(n),
        schema: schema(),
        root_layer: "root.usda".into(),
        entries: layer_set(n)
            .iter()
            .map(|(name, role, bytes)| LayerEntry::from_bytes(name, *role, bytes).unwrap())
            .collect(),
    }
}

struct Crash;

/// The commit unwinds at `at`: nothing after that point runs, as in a crash.
fn crash_commit(fx: &Fixture, d: &RevisionDraft, base: &str, at: CommitPoint) {
    let expected = if base == "none" {
        ExpectedBase::Initial
    } else {
        ExpectedBase::Revision(Uuid7::parse(base).unwrap())
    };
    let store = fx.revisions();
    let result = catch_unwind(AssertUnwindSafe(|| {
        store.commit_with(
            d,
            &expected,
            None,
            &mut |p: CommitPoint| -> Result<(), Injected> {
                if p == at {
                    resume_unwind(Box::new(Crash));
                }
                Ok(())
            },
        )
    }));
    assert!(result.is_err());
}

#[test]
fn a_cli_commit_writes_the_ledger_and_revision_outcome_answers_from_it() {
    let fx = Fixture::new();
    let store = fx.store_arg();
    // Nothing recorded: the ledger would cover the whole (empty) store, so absence is proven.
    let (exit, body) = fx.outcome(OP1);
    assert_eq!(
        (exit, text(&body, "outcome").as_str()),
        (0, "outcome_not_recorded")
    );
    assert!(!fx.ledger().exists());

    blob_put(&fx, 1);
    let (exit, first) = commit(&fx, 1, REV1, OP1, "none");
    assert_eq!(exit, 0, "{first:?}");
    assert!(fx.ledger().exists());
    let (exit, body) = fx.outcome(OP1);
    assert_eq!(exit, 0, "{body:?}");
    assert_eq!(text(&body, "outcome"), "outcome_committed");
    assert_eq!(text(&body, "source"), "ledger");
    assert_eq!(text(&body, "op_id"), OP1);
    assert_eq!(text(&body, "revision_id"), REV1);
    assert_eq!(text(&body, "sequence"), "1");
    assert_eq!(
        text(&body, "manifest_digest"),
        text(&first, "manifest_digest")
    );
    let (exit, body) = fx.outcome(OTHER);
    assert_eq!(
        (exit, text(&body, "outcome").as_str()),
        (0, "outcome_not_recorded")
    );

    // Bad input is rejected with the existing exit code and no hidden default.
    for args in [
        vec!["inspect-revision-outcome", "--store", &store],
        vec![
            "inspect-revision-outcome",
            "--store",
            &store,
            "--op-id",
            "1",
        ],
        vec!["recover-revision", "--store", &store],
        vec![
            "recover-revision",
            "--store",
            &store,
            "--min-age-secs",
            "01",
        ],
        vec![
            "recover-revision",
            "--store",
            &store,
            "--min-age-secs",
            "-1",
        ],
        vec!["recover-revision", "--store", &store, "--min-age-secs", ""],
    ] {
        let (exit, body) = fx.run(&args);
        assert_eq!(exit, 2, "{args:?}");
        assert_eq!(text(&body, "outcome"), "rejected", "{args:?}");
    }
}

#[test]
fn an_interrupted_commit_is_resolved_by_the_binary_at_every_crash_point() {
    // (point, did the root switch?, is the intent left unresolved?)
    let cases = [
        (CommitPoint::Checked, false, false),
        (CommitPoint::IntentRecorded, false, true),
        (CommitPoint::BlobsVerified, false, true),
        (CommitPoint::ManifestStored, false, true),
        (CommitPoint::RecordStaged, false, true),
        (CommitPoint::Switched, true, true),
        (CommitPoint::Recorded, true, false),
    ];
    for (point, switched, open) in cases {
        let fx = Fixture::new();
        let store = fx.store_arg();
        blob_put(&fx, 1);
        assert_eq!(commit(&fx, 1, REV1, OP1, "none").0, 0);
        blob_put(&fx, 2);
        crash_commit(&fx, &draft(2, REV2, OP2), REV1, point);
        let label = format!("{point:?}");
        let head = if switched { "2" } else { "1" };

        // A fresh process sees one complete revision, whatever the crash point was.
        let (exit, opened) = fx.run(&["open-revision", "--store", &store]);
        assert_eq!(exit, 0, "{label}: {opened:?}");
        assert_eq!(text(&opened, "sequence"), head, "{label}");

        // QueryOutcome never says more than the root and the ledger prove.
        let (exit, before) = fx.outcome(OP2);
        match (open, switched) {
            (true, false) => {
                assert_eq!(exit, 4, "{label}: {before:?}");
                assert_eq!(text(&before, "outcome"), "outcome_unknown");
                assert_eq!(text(&before, "code"), "OUTCOME_UNKNOWN");
            }
            (true, true) => assert_eq!(text(&before, "source"), "root", "{label}"),
            (false, true) => assert_eq!(text(&before, "source"), "ledger", "{label}"),
            (false, false) => {
                assert_eq!(text(&before, "outcome"), "outcome_not_recorded", "{label}");
            }
        }

        let (exit, report) = fx.recover("0");
        assert_eq!(exit, 0, "{label}: {report:?}");
        assert_eq!(text(&report, "outcome"), "recovery_complete", "{label}");
        assert_eq!(
            ids(&report, "committed"),
            if open && switched { vec![OP2] } else { vec![] },
            "{label}"
        );
        assert_eq!(
            ids(&report, "aborted"),
            if open && !switched { vec![OP2] } else { vec![] },
            "{label}"
        );
        assert!(ids(&report, "in_flight").is_empty() && ids(&report, "disagreements").is_empty());
        assert_eq!(report.get("root_error").unwrap(), &Json::Null);
        let revision = report.get("revision").unwrap();
        assert_eq!(text(revision, "outcome"), "revision_verified");
        assert_eq!(text(revision, "sequence"), head, "{label}");
        assert!(report.get("unreferenced").unwrap().get("total").is_ok());

        // The ledger now agrees with the root.
        let (exit, after) = fx.outcome(OP2);
        assert_eq!(exit, 0, "{label}: {after:?}");
        if switched {
            assert_eq!(text(&after, "outcome"), "outcome_committed", "{label}");
            assert_eq!(text(&after, "source"), "ledger");
            assert_eq!(text(&after, "revision_id"), REV2);
        } else if open {
            assert_eq!(text(&after, "outcome"), "outcome_aborted", "{label}");
            assert_eq!(text(&after, "reason"), "recovered");
        } else {
            assert_eq!(text(&after, "outcome"), "outcome_not_recorded", "{label}");
        }

        // A second run is a no-op: nothing closed, nothing removed, same ledger bytes.
        let ledger_bytes = fs::read(fx.ledger()).unwrap();
        let (exit, again) = fx.recover("0");
        assert_eq!(exit, 0);
        assert!(ids(&again, "committed").is_empty() && ids(&again, "aborted").is_empty());
        assert_eq!(text(&again, "removed_temp_files"), "0");
        assert_eq!(fs::read(fx.ledger()).unwrap(), ledger_bytes, "{label}");

        // The retry: replayed to the recorded revision if the root switched, otherwise it
        // commits exactly once.
        let (exit, retry) = commit(&fx, 2, REV2, OP2, REV1);
        assert_eq!(exit, 0, "{label}: {retry:?}");
        assert_eq!(
            text(&retry, "outcome"),
            if switched {
                "revision_replayed"
            } else {
                "revision_committed"
            },
            "{label}"
        );
        assert_eq!(
            (text(&retry, "sequence"), text(&retry, "revision_id")),
            ("2".into(), REV2.into())
        );
        let (exit, now) = fx.run(&["open-revision", "--store", &store]);
        assert_eq!((exit, text(&now, "sequence").as_str()), (0, "2"));
        assert_eq!(text(&fx.outcome(OP2).1, "source"), "ledger");
    }
}

#[test]
fn a_torn_ledger_tail_is_reported_and_recovery_rebuilds_the_record_from_the_root() {
    let fx = Fixture::new();
    blob_put(&fx, 1);
    assert_eq!(commit(&fx, 1, REV1, OP1, "none").0, 0);
    // Cut the Committed record of the last commit in half: the file now ends mid-record.
    let bytes = fs::read(fx.ledger()).unwrap();
    let start = bytes[..bytes.len() - 1]
        .iter()
        .rposition(|b| *b == b'\n')
        .unwrap()
        + 1;
    fs::write(fx.ledger(), &bytes[..start + (bytes.len() - start) / 2]).unwrap();

    // The torn record is never trusted, but the root still proves the operation.
    let (exit, body) = fx.outcome(OP1);
    assert_eq!(
        (exit, text(&body, "source").as_str()),
        (0, "root"),
        "{body:?}"
    );
    let (exit, report) = fx.recover("0");
    assert_eq!(exit, 0, "{report:?}");
    assert_eq!(ids(&report, "committed"), vec![OP1]);
    assert_eq!(
        report.get("ledger").unwrap().get("torn_tail").unwrap(),
        &Json::Bool(true)
    );
    assert_eq!(text(&fx.outcome(OP1).1, "source"), "ledger");
    let (_, again) = fx.recover("0");
    assert_eq!(
        again.get("ledger").unwrap().get("torn_tail").unwrap(),
        &Json::Bool(false)
    );
    assert_eq!(text(report.get("ledger").unwrap(), "damaged_lines"), "0");
    assert_eq!(text(again.get("ledger").unwrap(), "damaged_lines"), "1");
}

#[test]
fn a_young_intent_is_left_alone_until_the_operator_asserts_a_crash() {
    let fx = Fixture::new();
    blob_put(&fx, 1);
    crash_commit(&fx, &draft(1, REV1, OP1), "none", CommitPoint::RecordStaged);
    let (exit, report) = fx.recover("3600");
    assert_eq!(exit, 0, "{report:?}");
    assert_eq!(text(&report, "outcome"), "recovery_incomplete");
    assert_eq!(ids(&report, "in_flight"), vec![OP1]);
    assert!(ids(&report, "aborted").is_empty());
    assert_eq!(
        fx.outcome(OP1).0,
        4,
        "still unknown while it may be running"
    );
    let (_, report) = fx.recover("0");
    assert_eq!(ids(&report, "aborted"), vec![OP1]);
    assert_eq!(text(&report, "outcome"), "recovery_complete");
}

#[test]
fn a_ledger_that_contradicts_the_root_is_an_integrity_error_not_an_acceptance() {
    let fx = Fixture::new();
    blob_put(&fx, 1);
    assert_eq!(commit(&fx, 1, REV1, OP1, "none").0, 0);
    let forged = Record::Committed(CommittedRecord {
        op: Uuid7::parse(OTHER).unwrap(),
        revision: Uuid7::parse(REV2).unwrap(),
        sequence: 2,
        manifest: Sha256::digest(b"forged").unwrap(),
    })
    .encode()
    .0;
    let mut bytes = fs::read(fx.ledger()).unwrap();
    bytes.extend(forged);
    fs::write(fx.ledger(), bytes).unwrap();

    let (exit, body) = fx.outcome(OTHER);
    assert_eq!(exit, 2, "{body:?}");
    assert_eq!(text(&body, "code"), "INTEGRITY_ERROR");
    let (exit, report) = fx.recover("0");
    assert_eq!(exit, 0);
    assert_eq!(text(&report, "outcome"), "recovery_incomplete");
    assert_eq!(ids(&report, "disagreements"), vec![OTHER]);
    // The honest operation and the root are unaffected.
    assert_eq!(fx.outcome(OP1).0, 0);
    let (exit, opened) = fx.run(&["open-revision", "--store", &fx.store_arg()]);
    assert_eq!((exit, text(&opened, "sequence").as_str()), (0, "1"));
}
