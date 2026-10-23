// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic crash, race and refusal fixtures, MPL-2.0.

//! C06: actual storage interruption, historical replay, competing draft and publication guards.

use super::*;
use beamcraft_cli::compensation::runtime::{CompensationRequest, CompensationResult, compensate};
use beamcraft_host::revision::{CommitHook, Injected, NoFault};

fn prepared() -> (Fixture, RevisionInfo, RevisionInfo) {
    let f = Fixture::new(false);
    f.update("a", 10, &["--name", "accepted original"]);
    f.publish("a");
    let original = f.head("published");
    f.ok(&[
        "workspace-create",
        "--store",
        f.s("published"),
        "--workspace-dir",
        f.path("c").to_str().unwrap(),
        "--user-id",
        &id(3),
    ]);
    let draft = f.head("c");
    (f, original, draft)
}
fn request(original: &RevisionInfo, draft: &RevisionInfo) -> CompensationRequest {
    CompensationRequest {
        op_id: OpId::parse(&id(500)).unwrap(),
        actor: Uuid7::parse(&id(3)).unwrap(),
        original: original.revision().clone(),
        expected_head: original.revision().clone(),
        expected_draft: draft.revision().clone(),
    }
}
fn call(
    f: &Fixture,
    request: &CompensationRequest,
    hook: &mut dyn CommitHook,
) -> Result<CompensationResult, plate::PlateError> {
    compensate(
        &store(&f.path("published")),
        &store(&f.path("c")),
        &f.path("c"),
        &store(&f.path("a")),
        &f.path("a"),
        request,
        hook,
    )
}
fn retry(f: &Fixture, request: &CompensationRequest) -> (i32, Json) {
    f.run(&[
        "compensate-operation",
        "--store",
        f.s("published"),
        "--workspace-dir",
        f.path("c").to_str().unwrap(),
        "--source-workspace-dir",
        f.s("a"),
        "--op-id",
        &request.op_id.to_string(),
        "--original-revision",
        request.original.as_str(),
        "--expected-head",
        request.expected_head.as_str(),
        "--expected-draft",
        request.expected_draft.as_str(),
    ])
}

#[test]
fn interrupted_compensation_retries_once_and_keeps_a_later_local_edit() {
    for point in [CommitPoint::Checked, CommitPoint::Switched] {
        let (f, original, draft) = prepared();
        let mut request = request(&original, &draft);
        let metadata = fs::read(f.path("c").join("workspace.json")).unwrap();
        let mut reached = false;
        let attempted = call(&f, &request, &mut |seen| {
            if seen == point {
                reached = true;
                Err(Injected)
            } else {
                Ok(())
            }
        });
        assert!(reached);
        let error = attempted.err().unwrap();
        if point == CommitPoint::Switched {
            assert_eq!(error.code, ErrorCode::OutcomeUnknown);
            f.update("c", 11, &["--name", "later local"]);
        } else {
            assert_eq!(f.head("c").manifest_digest(), draft.manifest_digest());
        }
        let (code, result) = retry(&f, &request);
        assert_eq!(code, 0, "{result:?}");
        assert_eq!(
            text(&result, "outcome"),
            if point == CommitPoint::Switched {
                "compensation_replayed"
            } else {
                "operation_compensated"
            }
        );
        let head = f.head("c");
        assert_eq!(
            head.sequence(),
            draft.sequence() + if point == CommitPoint::Switched { 2 } else { 1 }
        );
        assert_eq!(
            text(result.get("revision").unwrap(), "revision_id"),
            id(500)
        );
        assert_eq!(
            fs::read(f.path("c").join("workspace.json")).unwrap(),
            metadata
        );
        assert_eq!(
            f.head("published").manifest_digest(),
            original.manifest_digest()
        );
        assert_eq!(retry(&f, &request).0, 0);
        assert_eq!(f.head("c").manifest_digest(), head.manifest_digest());
        request.expected_draft = head.revision().clone();
        let (code, result) = retry(&f, &request);
        assert_eq!(code, 2);
        assert_eq!(text(&result, "code"), "OP_ID_REUSE");
        f.ok(&[
            "workspace-publish",
            "--store",
            f.s("published"),
            "--workspace-dir",
            f.path("c").to_str().unwrap(),
        ]);
        let report = plate::inspect(&store(&f.path("published"))).unwrap();
        for (n, name) in [
            (10, "Plate"),
            (
                11,
                if point == CommitPoint::Switched {
                    "later local"
                } else {
                    "Plate"
                },
            ),
        ] {
            assert_eq!(
                report
                    .plates
                    .iter()
                    .find(|p| p.authored.id == ElementId::parse(&id(n)).unwrap())
                    .unwrap()
                    .authored
                    .name,
                name
            );
        }
    }
}

#[test]
fn a_compensating_draft_cas_loses_to_a_real_edit_while_accepted_publication_is_pinned() {
    let (f, original, draft) = prepared();
    f.ok(&[
        "workspace-create",
        "--store",
        f.s("published"),
        "--workspace-dir",
        f.path("other").to_str().unwrap(),
        "--user-id",
        &id(4),
    ]);
    f.update("other", 11, &["--name", "competing publication"]);
    let metadata = fs::read(f.path("c").join("workspace.json")).unwrap();
    let request = request(&original, &draft);
    let mut reached = false;
    let result = call(&f, &request, &mut |point| {
        if point == CommitPoint::Checked && !reached {
            reached = true;
            let (code, busy) = f.run(&[
                "workspace-publish",
                "--store",
                f.s("published"),
                "--workspace-dir",
                f.path("other").to_str().unwrap(),
            ]);
            assert_eq!(code, 3, "{busy:?}");
            assert_eq!(text(&busy, "code"), "BACKEND_UNAVAILABLE");
            f.update("c", 11, &["--name", "winning draft edit"]);
        }
        Ok(())
    });
    assert!(reached);
    assert_eq!(result.err().unwrap().code, ErrorCode::BaseRevisionMismatch);
    let winner = f.head("c");
    assert_eq!(winner.sequence(), draft.sequence() + 1);
    assert_eq!(
        f.head("published").manifest_digest(),
        original.manifest_digest()
    );
    assert_eq!(
        fs::read(f.path("c").join("workspace.json")).unwrap(),
        metadata
    );
    let report = plate::inspect(&store(&f.path("c"))).unwrap();
    for (n, name) in [(10, "accepted original"), (11, "winning draft edit")] {
        assert_eq!(
            report
                .plates
                .iter()
                .find(|p| p.authored.id == ElementId::parse(&id(n)).unwrap())
                .unwrap()
                .authored
                .name,
            name
        );
    }
    let (code, refused) = retry(&f, &request);
    assert_eq!(code, 2);
    assert_eq!(text(&refused, "code"), "BASE_REVISION_MISMATCH");
    assert_eq!(f.head("c").manifest_digest(), winner.manifest_digest());
}

#[test]
fn ownership_and_unsent_work_refuse_without_losing_the_local_model() {
    let (f, original, draft) = prepared();
    let mut request = request(&original, &draft);
    request.actor = Uuid7::parse(&id(4)).unwrap();
    assert_eq!(
        call(&f, &request, &mut NoFault).err().unwrap().code,
        ErrorCode::PermissionDenied
    );
    assert!(
        store(&f.path("c"))
            .changeset_claim(&Uuid7::parse(&id(500)).unwrap())
            .unwrap()
            .is_none()
    );
    request.actor = Uuid7::parse(&id(3)).unwrap();
    f.update("c", 11, &["--name", "retain unsent"]);
    let head = f.head("c");
    request.expected_draft = head.revision().clone();
    let metadata = fs::read(f.path("c").join("workspace.json")).unwrap();
    assert_eq!(
        call(&f, &request, &mut NoFault).err().unwrap().code,
        ErrorCode::InvalidArgument
    );
    assert_eq!(f.head("c").manifest_digest(), head.manifest_digest());
    assert_eq!(
        f.head("published").manifest_digest(),
        original.manifest_digest()
    );
    assert_eq!(
        fs::read(f.path("c").join("workspace.json")).unwrap(),
        metadata
    );
}

#[test]
fn the_original_workspace_can_also_be_the_compensation_destination() {
    let (f, original, _) = prepared();
    let draft = f.head("a");
    let args = [
        "compensate-operation",
        "--store",
        f.s("published"),
        "--workspace-dir",
        f.s("a"),
        "--source-workspace-dir",
        f.s("a"),
        "--original-revision",
        original.revision().as_str(),
        "--op-id",
        &id(500),
        "--expected-head",
        original.revision().as_str(),
        "--expected-draft",
        draft.revision().as_str(),
    ];
    let (code, result) = f.run(&args);
    assert_eq!(code, 0, "{result:?}");
    assert_eq!(
        f.head("published").manifest_digest(),
        original.manifest_digest()
    );
    f.publish("a");
    let (code, replay) = f.run(&args);
    assert_eq!(code, 0, "{replay:?}");
    assert_eq!(text(&replay, "outcome"), "compensation_replayed");
    let report = plate::inspect(&store(&f.path("published"))).unwrap();
    assert_eq!(
        report
            .plates
            .iter()
            .find(|p| p.authored.id == ElementId::parse(&id(10)).unwrap())
            .unwrap()
            .authored
            .name,
        "Plate"
    );
}

#[test]
fn parked_compensation_child() {
    let Some(root) = std::env::var_os("BEAMCRAFT_COMPENSATION_KILL_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let paths = ["published", "a", "b", "reference.usda"]
        .into_iter()
        .map(|name| (name, root.join(name).to_str().unwrap().to_owned()))
        .collect();
    let f = Fixture(root, paths);
    let original = f.head("published");
    let draft = f.head("c");
    let point = match std::env::var("BEAMCRAFT_COMPENSATION_KILL_POINT")
        .unwrap()
        .as_str()
    {
        "checked" => CommitPoint::Checked,
        "switched" => CommitPoint::Switched,
        _ => panic!("unknown test boundary"),
    };
    let _ = call(&f, &request(&original, &draft), &mut |seen| {
        if seen == point {
            fs::write(f.path("compensation-ready"), b"ready").unwrap();
            loop {
                std::thread::park();
            }
        }
        Ok(())
    });
    panic!("compensation did not reach its kill boundary");
}

#[test]
fn killed_compensation_processes_recover_one_complete_inverse() {
    use std::{
        process::Stdio,
        time::{Duration, Instant},
    };
    for point in ["checked", "switched"] {
        let (f, original, draft) = prepared();
        let request = request(&original, &draft);
        let metadata = fs::read(f.path("c").join("workspace.json")).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "compensation_recovery::parked_compensation_child",
                "--nocapture",
            ])
            .env("BEAMCRAFT_COMPENSATION_KILL_ROOT", &f.0)
            .env("BEAMCRAFT_COMPENSATION_KILL_POINT", point)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let started = Instant::now();
        let reached = loop {
            if f.path("compensation-ready").is_file() {
                break true;
            }
            if child.try_wait().unwrap().is_some() || started.elapsed() > Duration::from_secs(20) {
                break false;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        // Always reap the isolated test writer, including a failed readiness assertion.
        let _ = child.kill();
        child.wait().unwrap();
        assert!(reached, "child did not reach {point}");
        assert_eq!(
            f.head("published").manifest_digest(),
            original.manifest_digest()
        );
        assert_eq!(
            fs::read(f.path("c").join("workspace.json")).unwrap(),
            metadata
        );
        // Zero grace is valid here: the only writer was just killed and reaped.
        f.ok(&[
            "revision-recover",
            "--store",
            f.path("c").to_str().unwrap(),
            "--min-age-secs",
            "0",
        ]);
        let (code, result) = retry(&f, &request);
        assert_eq!(code, 0, "{point}: {result:?}");
        assert_eq!(
            text(&result, "outcome"),
            if point == "switched" {
                "compensation_replayed"
            } else {
                "operation_compensated"
            }
        );
        assert_eq!(f.head("c").sequence(), draft.sequence() + 1);
        assert_eq!(
            f.head("published").manifest_digest(),
            original.manifest_digest()
        );
        assert_eq!(
            fs::read(f.path("c").join("workspace.json")).unwrap(),
            metadata
        );
        let report = plate::inspect(&store(&f.path("c"))).unwrap();
        assert_eq!(report.plates.len(), 2);
        assert_eq!(
            report
                .plates
                .iter()
                .find(|p| p.authored.id == ElementId::parse(&id(10)).unwrap())
                .unwrap()
                .authored
                .name,
            "Plate"
        );
        f.ok(&[
            "workspace-publish",
            "--store",
            f.s("published"),
            "--workspace-dir",
            f.path("c").to_str().unwrap(),
        ]);
    }
}

#[test]
fn a_stale_workspace_is_not_silently_replaced_by_the_current_accepted_basis() {
    let (f, original, draft) = prepared();
    f.ok(&[
        "workspace-create",
        "--store",
        f.s("published"),
        "--workspace-dir",
        f.path("other").to_str().unwrap(),
        "--user-id",
        &id(4),
    ]);
    f.update("other", 11, &["--name", "new accepted"]);
    f.ok(&[
        "workspace-publish",
        "--store",
        f.s("published"),
        "--workspace-dir",
        f.path("other").to_str().unwrap(),
    ]);
    let accepted = f.head("published");
    let mut request = request(&original, &draft);
    request.expected_head = accepted.revision().clone();
    let metadata = fs::read(f.path("c").join("workspace.json")).unwrap();
    assert_eq!(
        call(&f, &request, &mut NoFault).err().unwrap().code,
        ErrorCode::BaseRevisionMismatch
    );
    assert_eq!(f.head("c").manifest_digest(), draft.manifest_digest());
    assert_eq!(
        f.head("published").manifest_digest(),
        accepted.manifest_digest()
    );
    assert_eq!(
        fs::read(f.path("c").join("workspace.json")).unwrap(),
        metadata
    );
}

#[test]
fn restoring_a_deleted_part_never_replaces_another_users_new_part_with_the_same_id() {
    use beamcraft_engine::footprint::{Group, Target};
    let f = Fixture::new(false);
    f.ok(&["delete-element", "--store", f.s("a"), "--id", &id(10)]);
    f.publish("a");
    let original = f.head("published");
    f.ok(&[
        "workspace-create",
        "--store",
        f.s("published"),
        "--workspace-dir",
        f.path("other").to_str().unwrap(),
        "--user-id",
        &id(4),
    ]);
    f.ok(&[
        "create-plate",
        "--store",
        f.path("other").to_str().unwrap(),
        "--new-id",
        &id(10),
        "--width",
        "20",
        "--length",
        "30",
        "--thickness",
        "5",
        "--name",
        "Replacement",
    ]);
    f.ok(&[
        "workspace-publish",
        "--store",
        f.s("published"),
        "--workspace-dir",
        f.path("other").to_str().unwrap(),
    ]);
    let accepted = f.head("published");
    f.ok(&[
        "workspace-create",
        "--store",
        f.s("published"),
        "--workspace-dir",
        f.path("c").to_str().unwrap(),
        "--user-id",
        &id(3),
    ]);
    let draft = f.head("c");
    let mut request = request(&original, &draft);
    request.expected_head = accepted.revision().clone();
    let metadata = fs::read(f.path("c").join("workspace.json")).unwrap();
    let result = call(&f, &request, &mut NoFault).unwrap();
    let CompensationResult::Conflicted(conflicts) = result else {
        panic!("restored over a later user's part")
    };
    assert!(conflicts.iter().any(|c| c.entry.group == Group::Lifecycle
        && c.entry.target == Target::Element(ElementId::parse(&id(10)).unwrap())));
    assert_eq!(f.head("c").manifest_digest(), draft.manifest_digest());
    assert_eq!(
        f.head("published").manifest_digest(),
        accepted.manifest_digest()
    );
    assert_eq!(
        fs::read(f.path("c").join("workspace.json")).unwrap(),
        metadata
    );
    let report = plate::inspect(&store(&f.path("c"))).unwrap();
    let replacement = report
        .plates
        .iter()
        .find(|p| p.authored.id == ElementId::parse(&id(10)).unwrap())
        .unwrap();
    assert_eq!(replacement.authored.name, "Replacement");
    assert_eq!(replacement.quantities.volume_mm3, 3000.0);
}
