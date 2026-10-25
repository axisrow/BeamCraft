// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! U06 headless revision commands over a protected store directory. They call
//! `beamcraft_host::revision` and parse no USD: layer content checks need the layer engine,
//! which runs in its own process and is not wired to these commands (see the README).

use beamcraft_host::{
    control::{Control, HostError},
    ledger::{Outcome, OutcomeSource, RecoveryPolicy},
    revision::{
        CommitOutcome, CommitPoint, ExpectedBase, Injected, LayerEntry, LayerRole, RevisionDraft,
        RevisionError, RevisionInfo, RevisionStore, Uuid7,
    },
    storage::BlobStore,
};
use beamcraft_json::Json;
use beamcraft_model::{ErrorCode, digest::ContentDigest};
use std::collections::BTreeMap;

/// The shell argument cap (4096 bytes) already bounds the list; this is a clearer limit.
const MAX_CLI_LAYERS: usize = 16;

fn invalid() -> HostError {
    HostError::logical(ErrorCode::InvalidArgument)
}

fn failure(error: RevisionError) -> HostError {
    HostError::logical(error.code())
}

/// Canonical unsigned decimal, as the logical `WorkspaceGeneration` form requires.
fn generation(text: &str) -> Option<u64> {
    let canonical = !text.is_empty()
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    if canonical { text.parse().ok() } else { None }
}

fn summary(outcome: &str, info: &RevisionInfo) -> Json {
    let manifest = info.manifest();
    let content = manifest.content();
    let layers = content
        .entries()
        .iter()
        .map(|e| {
            Json::object([
                ("role", Json::text(e.role().as_str())),
                ("name", Json::text(e.name())),
                ("digest", Json::text(e.digest().to_string())),
                ("size", Json::text(e.size().to_string())),
            ])
        })
        .collect();
    Json::object([
        ("outcome", Json::text(outcome)),
        ("revision_id", Json::text(info.revision().as_str())),
        ("sequence", Json::text(info.sequence().to_string())),
        (
            "manifest_digest",
            Json::text(info.manifest_digest().to_string()),
        ),
        ("project_id", Json::text(content.project().as_str())),
        ("workspace_id", Json::text(content.workspace().as_str())),
        ("op_id", Json::text(content.op().as_str())),
        (
            "workspace_generation",
            Json::text(content.workspace_generation().to_string()),
        ),
        ("schema", Json::text(content.schema().to_string())),
        ("root_layer", Json::text(content.root_layer())),
        (
            "base",
            manifest.base().map_or(Json::Null, |b| {
                Json::object([
                    ("revision_id", Json::text(b.revision.as_str())),
                    ("manifest_digest", Json::text(b.manifest.to_string())),
                ])
            }),
        ),
        (
            "index",
            manifest.index().map_or(Json::Null, |i| {
                Json::object([
                    ("structure_digest", Json::text(i.structure.to_string())),
                    ("prim_count", Json::text(i.prims.to_string())),
                ])
            }),
        ),
        ("layers", Json::Array(layers)),
    ])
}

/// `commit-revision`: all layer blobs must already be in the store (`put-blob`). Each
/// `--layers` item is `ROLE:NAME:SHA256`; the size is taken from the verified stored bytes.
/// The deadline and cancellation of `control` are honoured while preparing and at every commit
/// point before the root switch; they are never checked after it.
pub(crate) fn commit(
    blobs: &BlobStore,
    options: &BTreeMap<&str, &str>,
    control: &Control,
) -> Result<Json, HostError> {
    commit_checked(blobs, options, &mut || control.remaining().map(|_| ()))
}

fn commit_checked(
    blobs: &BlobStore,
    options: &BTreeMap<&str, &str>,
    check: &mut dyn FnMut() -> Result<(), HostError>,
) -> Result<Json, HostError> {
    let get = |key: &str| options.get(key).copied().ok_or_else(invalid);
    let uuid = |key: &str| Uuid7::parse(get(key)?).map_err(failure);
    let digest = |key: &str| get(key)?.parse::<ContentDigest>().map_err(|_| invalid());
    check()?;
    let mut entries = Vec::new();
    for item in get("--layers")?.split(',') {
        if entries.len() >= MAX_CLI_LAYERS {
            return Err(HostError::logical(ErrorCode::LimitExceeded));
        }
        let mut parts = item.split(':');
        let (Some(role), Some(name), Some(layer), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(invalid());
        };
        let layer = layer.parse::<ContentDigest>().map_err(|_| invalid())?;
        check()?;
        let size = blobs
            .read(&layer)
            .map_err(|e| HostError::logical(e.code()))?
            .len();
        entries.push(
            LayerEntry::new(
                name,
                LayerRole::parse(role).ok_or_else(invalid)?,
                layer,
                u64::try_from(size).map_err(|_| invalid())?,
            )
            .map_err(failure)?,
        );
    }
    let expected = match get("--base")? {
        "none" => ExpectedBase::Initial,
        id => ExpectedBase::Revision(Uuid7::parse(id).map_err(failure)?),
    };
    let draft = RevisionDraft {
        project: uuid("--project-id")?,
        workspace: uuid("--workspace-id")?,
        revision: uuid("--revision-id")?,
        op: uuid("--op-id")?,
        workspace_generation: generation(get("--generation")?).ok_or_else(invalid)?,
        schema: digest("--schema")?,
        root_layer: get("--root-layer")?.to_owned(),
        entries,
    };
    // A failed check aborts at the next pre-switch point with the root unchanged and reports the
    // original cancellation or deadline error. `Switched` is exempt: after it the revision is
    // committed and a timeout would hide it.
    let mut stopped = None;
    let result = RevisionStore::new(blobs.clone()).commit_with(
        &draft,
        &expected,
        None,
        &mut |point: CommitPoint| {
            if matches!(point, CommitPoint::Switched | CommitPoint::Recorded) {
                return Ok(());
            }
            check().map_err(|error| {
                stopped = Some(error);
                Injected
            })
        },
    );
    let committed = match (result, stopped) {
        (Err(RevisionError::Aborted(_)), Some(error)) => return Err(error),
        (result, _) => result.map_err(failure)?,
    };
    Ok(summary(
        match committed.outcome {
            CommitOutcome::Created => "revision_committed",
            CommitOutcome::Replayed => "revision_replayed",
        },
        &committed.info,
    ))
}

/// `open-revision`: opens the current root and verifies the manifest and every layer blob.
pub(crate) fn open(blobs: &BlobStore) -> Result<Json, HostError> {
    let revision = RevisionStore::new(blobs.clone()).open().map_err(failure)?;
    Ok(summary("revision_verified", revision.info()))
}

/// Most unreferenced digests `recover-revision` prints; the total is always exact.
const MAX_CLI_UNREFERENCED: usize = 256;

fn uuids(ids: &[Uuid7]) -> Json {
    Json::Array(ids.iter().map(|id| Json::text(id.as_str())).collect())
}

/// `recover-revision`: closes every unresolved intent from the root, removes abandoned
/// temporary files, reports unreferenced blobs (never deletes one) and verifies the root.
/// `--min-age-secs 0` asserts that no commit is running on the store; a larger value leaves
/// younger intents and files alone. Exit 0 with `recovery_incomplete` when something could not
/// be decided; the report says what.
pub(crate) fn recover(
    blobs: &BlobStore,
    options: &BTreeMap<&str, &str>,
) -> Result<Json, HostError> {
    let min_age = options
        .get("--min-age-secs")
        .copied()
        .and_then(generation)
        .ok_or_else(invalid)?;
    let store = RevisionStore::new(blobs.clone());
    let report = store
        .recover(RecoveryPolicy::live(min_age))
        .map_err(failure)?;
    let (revision, revision_error) = match store.open() {
        Ok(revision) => (summary("revision_verified", revision.info()), Json::Null),
        Err(RevisionError::NoRevision) => (Json::Null, Json::Null),
        Err(error) => (Json::Null, Json::text(error.code().as_str())),
    };
    let complete = report.unprovable.is_empty()
        && report.disagreements.is_empty()
        && report.in_flight.is_empty()
        && report.root_error.is_none()
        && revision_error == Json::Null;
    let unreferenced = report.unreferenced.as_ref().map_or(Json::Null, |u| {
        Json::object([
            ("total", Json::text(u.total.to_string())),
            (
                "digests",
                Json::Array(
                    u.digests
                        .iter()
                        .take(MAX_CLI_UNREFERENCED)
                        .map(|d| Json::text(d.to_string()))
                        .collect(),
                ),
            ),
        ])
    });
    Ok(Json::object([
        (
            "outcome",
            Json::text(if complete {
                "recovery_complete"
            } else {
                "recovery_incomplete"
            }),
        ),
        ("min_age_secs", Json::text(min_age.to_string())),
        ("committed", uuids(&report.committed)),
        ("aborted", uuids(&report.aborted)),
        ("in_flight", uuids(&report.in_flight)),
        (
            "unprovable",
            Json::Array(
                report
                    .unprovable
                    .iter()
                    .map(|(op, error)| {
                        Json::object([
                            ("op_id", Json::text(op.as_str())),
                            ("code", Json::text(error.code().as_str())),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("disagreements", uuids(&report.disagreements)),
        (
            "root_error",
            report
                .root_error
                .map_or(Json::Null, |e| Json::text(e.code().as_str())),
        ),
        (
            "removed_temp_files",
            Json::text(report.removed_temp_files.to_string()),
        ),
        ("unreferenced", unreferenced),
        (
            "ledger",
            Json::object([
                ("bytes", Json::text(report.ledger.bytes.to_string())),
                ("records", Json::text(report.ledger.records.to_string())),
                (
                    "damaged_lines",
                    Json::text(report.ledger.damaged_lines.to_string()),
                ),
                ("torn_tail", Json::Bool(report.ledger.torn_tail)),
            ]),
        ),
        ("revision", revision),
        ("revision_error", revision_error),
    ]))
}

/// `inspect-revision-outcome`: the host-ledger QueryOutcome for one op id. Read-only. A recorded
/// result is exit 0; `OUTCOME_UNKNOWN` is the standard unknown result (exit 4); a ledger that
/// contradicts the root is `INTEGRITY_ERROR`.
pub(crate) fn outcome(
    blobs: &BlobStore,
    options: &BTreeMap<&str, &str>,
) -> Result<Json, HostError> {
    let op = Uuid7::parse(options.get("--op-id").copied().ok_or_else(invalid)?).map_err(failure)?;
    let answer = RevisionStore::new(blobs.clone())
        .query_outcome(&op)
        .map_err(failure)?;
    Ok(match answer {
        Outcome::Committed {
            revision,
            sequence,
            manifest,
            source,
        } => Json::object([
            ("outcome", Json::text("outcome_committed")),
            ("op_id", Json::text(op.as_str())),
            ("revision_id", Json::text(revision.as_str())),
            ("sequence", Json::text(sequence.to_string())),
            ("manifest_digest", Json::text(manifest.to_string())),
            (
                "source",
                Json::text(match source {
                    OutcomeSource::Ledger => "ledger",
                    OutcomeSource::Root => "root",
                }),
            ),
        ]),
        Outcome::Aborted { reason } => Json::object([
            ("outcome", Json::text("outcome_aborted")),
            ("op_id", Json::text(op.as_str())),
            ("reason", Json::text(reason.as_str())),
        ]),
        Outcome::NotFound => Json::object([
            ("outcome", Json::text("outcome_not_recorded")),
            ("op_id", Json::text(op.as_str())),
        ]),
        Outcome::Unknown => return Err(HostError::logical(ErrorCode::OutcomeUnknown)),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Store(std::path::PathBuf);
    impl Store {
        fn new() -> Self {
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join("u06-unit")
                .join(format!(
                    "{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }
    impl Drop for Store {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn run(
        fail_at: Option<usize>,
    ) -> (
        usize,
        Result<Json, HostError>,
        Option<std::path::PathBuf>,
        Store,
    ) {
        let store = Store::new();
        let blobs = BlobStore::new(&store.0).unwrap();
        let root = blobs.publish(b"#usda 1.0\n# root\n").unwrap().0;
        let derived = blobs.publish(b"#usda 1.0\n# derived\n").unwrap().0;
        let layers = format!("authored:root.usda:{root},derived:derived.usda:{derived}");
        let schema = root.to_string();
        let options = BTreeMap::from([
            ("--layers", layers.as_str()),
            ("--project-id", "0199e7a0-0000-7000-8000-000000000001"),
            ("--workspace-id", "0199e7a0-0000-7000-8000-000000000002"),
            ("--revision-id", "0199e7a0-0000-7000-8000-000000000003"),
            ("--op-id", "0199e7a0-0000-7000-8000-000000000004"),
            ("--generation", "7"),
            ("--base", "none"),
            ("--schema", schema.as_str()),
            ("--root-layer", "root.usda"),
        ]);
        let mut calls = 0usize;
        let result = commit_checked(&blobs, &options, &mut || {
            calls += 1;
            if fail_at == Some(calls) {
                Err(HostError::logical(ErrorCode::BackendUnavailable))
            } else {
                Ok(())
            }
        });
        let log = store.0.join("revisions");
        (calls, result, log.exists().then_some(log), store)
    }

    #[test]
    fn a_failed_check_at_any_pre_switch_point_aborts_with_the_original_error_and_no_root() {
        let (total, ok, log, _keep) = run(None);
        assert!(ok.is_ok());
        assert!(log.is_some());
        // One check before the layer reads, one per layer read, one at each of the five
        // pre-switch points (U07 added `IntentRecorded`).
        assert_eq!(total, 8);
        for at in 1..=total {
            let (calls, result, log, store) = run(Some(at));
            let error = result.unwrap_err();
            // The deadline/cancellation error is preserved, not replaced by the abort code.
            assert_eq!(error.code, ErrorCode::BackendUnavailable, "check {at}");
            assert_eq!(calls, at, "no check runs after the failing one");
            // Nothing was published: no head record exists (the directory may hold none).
            let head =
                beamcraft_host::revision::RevisionStore::new(BlobStore::new(&store.0).unwrap())
                    .head()
                    .unwrap();
            assert_eq!(head, None, "check {at}");
            assert!(log.is_none_or(|l| std::fs::read_dir(l).unwrap().count() == 0));
        }
    }
}
