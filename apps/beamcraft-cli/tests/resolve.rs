// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original models and references, MPL-2.0; existing test-data attribution applies.

//! Actual two-client CLI conflict resolution (C04.2): discard, re-edit, replay and refusals.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_cli::{
    plate,
    workspace::resolve::{ResolveRequest, resolve_workspace},
};
use beamcraft_host::{
    revision::{
        CommitPoint, ExpectedBase, Injected, LayerEntry, RevisionDraft, RevisionInfo,
        RevisionStore, Uuid7,
    },
    storage::BlobStore,
};
use beamcraft_json::{Json, Limits};
use beamcraft_model::{
    error::ErrorCode,
    id::{ElementId, OpId},
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
fn id(n: usize) -> String {
    format!("019a7bf0-0000-7000-8000-{n:012x}")
}
fn store(path: &Path) -> RevisionStore {
    RevisionStore::new(BlobStore::new(path).unwrap())
}
fn text<'a>(json: &'a Json, key: &str) -> &'a str {
    json.get(key).unwrap().string().unwrap()
}
struct Fixture(PathBuf, BTreeMap<&'static str, String>);
impl Fixture {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "resolve-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let paths = ["published", "a", "b", "c"]
            .into_iter()
            .map(|name| (name, root.join(name).to_str().unwrap().to_owned()))
            .collect();
        let f = Self(root, paths);
        fs::create_dir(&f.0).unwrap();
        fs::create_dir(f.path("published")).unwrap();
        let material = f.ok(&[
            "define-material",
            "--store",
            f.s("published"),
            "--density",
            "7850",
        ]);
        let material = text(material.get("definition").unwrap(), "digest");
        for n in [10, 11] {
            f.ok(&[
                "create-plate",
                "--store",
                f.s("published"),
                "--width",
                "1000",
                "--length",
                "500",
                "--thickness",
                "10",
                "--new-id",
                &id(n),
                "--material-digest",
                material,
            ]);
        }
        for name in ["a", "b", "c"] {
            f.ok(&[
                "create-workspace",
                "--store",
                f.s("published"),
                "--workspace-dir",
                f.s(name),
                "--user-id",
                &id(3),
            ]);
        }
        f
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn s(&self, name: &str) -> &str {
        self.1.get(name).unwrap()
    }
    fn run(&self, args: &[&str]) -> (i32, Json) {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .output()
            .unwrap();
        (
            output.status.code().unwrap(),
            beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap(),
        )
    }
    fn ok(&self, args: &[&str]) -> Json {
        let (code, json) = self.run(args);
        assert_eq!(code, 0, "{args:?}: {json:?}");
        json
    }
    fn head(&self, name: &str) -> RevisionInfo {
        store(&self.path(name)).head().unwrap().unwrap()
    }
    fn op_at(&self, name: &str, sequence: u64) -> String {
        store(&self.path(name))
            .info_at(sequence)
            .unwrap()
            .manifest()
            .content()
            .op()
            .to_string()
    }
    fn update(&self, name: &str, n: usize, patch: &[&str]) {
        let path = self.path(name);
        let mut args = vec!["update-element", "--store", path.to_str().unwrap(), "--id"];
        let element = id(n);
        args.push(&element);
        args.extend(patch);
        self.ok(&args);
    }
    fn publish(&self, name: &str) {
        self.ok(&[
            "publish-workspace",
            "--store",
            self.s("published"),
            "--workspace-dir",
            self.s(name),
        ]);
    }
    fn rebase(
        &self,
        name: &str,
        n: usize,
        accepted: &RevisionInfo,
        draft: &RevisionInfo,
    ) -> (i32, Json) {
        self.run(&[
            "rebase-workspace",
            "--store",
            self.s("published"),
            "--workspace-dir",
            self.s(name),
            "--op-id",
            &id(n),
            "--expected-head",
            accepted.revision().as_str(),
            "--expected-draft",
            draft.revision().as_str(),
        ])
    }
    fn resolve(
        &self,
        name: &str,
        op: usize,
        discard: &str,
        expected: &RevisionInfo,
    ) -> (i32, Json) {
        self.run(&[
            "resolve-workspace",
            "--workspace-dir",
            self.s(name),
            "--op-id",
            &id(op),
            "--discard",
            discard,
            "--expected-draft",
            expected.revision().as_str(),
        ])
    }
    fn status_of(&self, name: &str) -> Json {
        self.ok(&[
            "inspect-workspace",
            "--store",
            self.s("published"),
            "--workspace-dir",
            self.s(name),
        ])
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn name_of(report: &beamcraft_cli::plate::Inspection, n: usize) -> String {
    report
        .plates
        .iter()
        .find(|p| p.authored.id == ElementId::parse(&id(n)).unwrap())
        .unwrap()
        .authored
        .name
        .clone()
}

fn pending(status: &Json) -> Vec<Json> {
    status
        .get("workspace")
        .unwrap()
        .get("pending_conflicts")
        .unwrap()
        .array()
        .unwrap()
        .to_vec()
}

/// Commits a draft revision that duplicates the current head's layers with no operation
/// evidence and no verifier: the shape of a legacy evidence-less undo revision.
fn commit_without_evidence(path: &Path, revision: &str, op: &str) {
    let s = store(path);
    let head = s.open().unwrap();
    let content = head.manifest().content();
    let mut entries = Vec::new();
    for entry in content.entries() {
        let bytes = head.layer_bytes(entry.name()).unwrap();
        entries.push(LayerEntry::from_bytes(entry.name(), entry.role(), &bytes).unwrap());
    }
    s.commit_with_evidence(
        &RevisionDraft {
            project: content.project().clone(),
            workspace: content.workspace().clone(),
            revision: Uuid7::parse(revision).unwrap(),
            op: Uuid7::parse(op).unwrap(),
            workspace_generation: content.workspace_generation() + 1,
            schema: content.schema(),
            root_layer: content.root_layer().to_owned(),
            entries,
        },
        &ExpectedBase::Revision(head.info().revision().clone()),
        None,
        None,
        &mut |_: CommitPoint| Ok(()),
    )
    .unwrap();
}

// C04-01: the discard resolves the recorded conflict, the gate lifts and the published value
// is the accepted one; the renounced operation never reappears in any later publication.
#[test]
fn a_discard_resolves_the_conflict_and_the_accepted_value_is_published() {
    let f = Fixture::new();
    let fork_name = name_of(&plate::inspect(&store(&f.path("published"))).unwrap(), 10);
    f.update("a", 10, &["--name", "remote name"]);
    f.update("b", 10, &["--name", "local name"]);
    f.publish("a");
    let accepted = f.head("published");
    let draft = f.head("b");
    let (code, result) = f.rebase("b", 500, &accepted, &draft);
    assert_eq!(code, 2, "{result:?}");
    assert_eq!(text(&result, "code"), "DEPENDENCY_CONFLICT");
    let discarded = f.op_at("b", draft.sequence());
    let (code, resolved) = f.resolve("b", 600, &discarded, &draft);
    assert_eq!(code, 0, "{resolved:?}");
    assert_eq!(text(&resolved, "outcome"), "workspace_resolved");
    assert_eq!(
        resolved
            .get("renounced_operations")
            .unwrap()
            .array()
            .unwrap()
            .iter()
            .map(|op| op.string().unwrap())
            .collect::<Vec<_>>(),
        vec![discarded.as_str()]
    );
    let resolved_head = f.head("b");
    assert_eq!(resolved_head.sequence(), draft.sequence() + 1);
    // Reading the status purges the stale record and reports nothing pending.
    assert!(pending(&f.status_of("b")).is_empty());
    assert!(!f.path("b").join("conflicts.pending").exists());
    // The draft content is the restored fork state: nothing of the renounced work is left.
    let draft_report = plate::inspect(&store(&f.path("b"))).unwrap();
    assert_eq!(name_of(&draft_report, 10), fork_name);
    // The conflict gate is gone; the base check still refuses a publication until a fresh
    // rebase merges the draft onto the newer accepted head.
    let publish = [
        "publish-workspace",
        "--store",
        f.s("published"),
        "--workspace-dir",
        f.s("b"),
    ];
    let (code, refused) = f.run(&publish);
    assert_eq!(code, 2);
    assert_eq!(text(&refused, "code"), "BASE_REVISION_MISMATCH");

    // Another actor publishes meanwhile; the resolution is timing independent.
    f.update("a", 11, &["--name", "a eleven"]);
    f.publish("a");
    let newer = f.head("published");
    let (code, rebased) = f.rebase("b", 502, &newer, &resolved_head);
    assert_eq!(code, 0, "{rebased:?}");
    assert_eq!(text(&rebased, "outcome"), "workspace_rebased");
    // A fully renounced draft ships nothing: the accepted side is already the predicted state
    // with A's value standing and the renounced work never having reappeared.
    let (code, refused) = f.run(&publish);
    assert_eq!(code, 2);
    assert_eq!(text(&refused, "code"), "INVALID_ARGUMENT");
    let final_report = plate::inspect(&store(&f.path("published"))).unwrap();
    assert_eq!(name_of(&final_report, 10), "remote name");
    assert_eq!(name_of(&final_report, 11), "a eleven");
    // A later edit still publishes normally, and the renounced value stays absent.
    f.update("b", 10, &["--name", "b again"]);
    f.publish("b");
    let final_report = plate::inspect(&store(&f.path("published"))).unwrap();
    assert_eq!(name_of(&final_report, 10), "b again");
    assert_eq!(name_of(&final_report, 11), "a eleven");
}

// C04-08: two clients hit the same published conflict; one discards, one discards and
// re-edits on the merged state. Each resolution is a new operation identity and both final
// states are the predicted ones.
#[test]
fn discard_and_re_edit_resolve_the_same_conflict_differently() {
    let f = Fixture::new();
    f.update("a", 10, &["--name", "remote name"]);
    f.update("b", 10, &["--name", "local b"]);
    f.update("c", 10, &["--name", "local c"]);
    f.publish("a");
    let accepted = f.head("published");
    for name in ["b", "c"] {
        let draft = f.head(name);
        let (code, result) = f.rebase(name, 500, &accepted, &draft);
        assert_eq!(code, 2, "{name}: {result:?}");
        assert_eq!(text(&result, "code"), "DEPENDENCY_CONFLICT");
    }
    // B renounces; C renounces and then re-edits with a new operation identity.
    let b_draft = f.head("b");
    let (code, b_resolved) = f.resolve("b", 600, &f.op_at("b", b_draft.sequence()), &b_draft);
    assert_eq!(code, 0, "{b_resolved:?}");
    let c_draft = f.head("c");
    let (code, c_resolved) = f.resolve("c", 601, &f.op_at("c", c_draft.sequence()), &c_draft);
    assert_eq!(code, 0, "{c_resolved:?}");
    assert_ne!(
        text(b_resolved.get("revision").unwrap(), "revision_id"),
        text(c_resolved.get("revision").unwrap(), "revision_id")
    );
    for name in ["b", "c"] {
        let head = f.head(name);
        let (code, rebased) = f.rebase(name, 510, &accepted, &head);
        assert_eq!(code, 0, "{name}: {rebased:?}");
    }
    f.update("c", 10, &["--name", "local c two"]);
    f.publish("c");
    assert_eq!(
        name_of(&plate::inspect(&store(&f.path("published"))).unwrap(), 10),
        "local c two"
    );
    // B's renunciation survives C's later publication untouched: B rebases onto it and ships
    // nothing of its own, removing no other actor's edit.
    let b_head = f.head("b");
    let current = f.head("published");
    let (code, rebased) = f.rebase("b", 512, &current, &b_head);
    assert_eq!(code, 0, "{rebased:?}");
    let (code, refused) = f.run(&[
        "publish-workspace",
        "--store",
        f.s("published"),
        "--workspace-dir",
        f.s("b"),
    ]);
    assert_eq!(code, 2);
    assert_eq!(text(&refused, "code"), "INVALID_ARGUMENT");
    let final_report = plate::inspect(&store(&f.path("published"))).unwrap();
    assert_eq!(name_of(&final_report, 10), "local c two");
}

// C04-10: the resolution is a recorded operation with its own identity: a fresh process
// replays the recorded outcome exactly once, and another payload under the identity is refused.
#[test]
fn the_resolution_replays_once_and_refuses_another_payload() {
    let f = Fixture::new();
    f.update("a", 10, &["--name", "remote name"]);
    f.update("b", 10, &["--name", "local name"]);
    f.publish("a");
    let accepted = f.head("published");
    let draft = f.head("b");
    let (code, _) = f.rebase("b", 500, &accepted, &draft);
    assert_eq!(code, 2);
    let discarded = f.op_at("b", draft.sequence());
    let (code, resolved) = f.resolve("b", 600, &discarded, &draft);
    assert_eq!(code, 0, "{resolved:?}");
    let head = f.head("b");
    let (code, replayed) = f.resolve("b", 600, &discarded, &draft);
    assert_eq!(code, 0, "{replayed:?}");
    assert_eq!(text(&replayed, "outcome"), "workspace_resolve_replayed");
    assert_eq!(
        text(replayed.get("revision").unwrap(), "revision_id"),
        text(resolved.get("revision").unwrap(), "revision_id")
    );
    assert_eq!(f.head("b").manifest_digest(), head.manifest_digest());
    let (code, reused) = f.resolve("b", 600, &id(999), &draft);
    assert_eq!(code, 2);
    assert_eq!(text(&reused, "code"), "OP_ID_REUSE");
    // The named operation is already renounced by the recorded resolution, so a second
    // resolution of it has no retained work to name.
    let (code, spent) = f.resolve("b", 601, &discarded, &head);
    assert_eq!(code, 2);
    assert_eq!(text(&spent, "code"), "INVALID_ARGUMENT");
    assert_eq!(f.head("b").manifest_digest(), head.manifest_digest());
}

// C04-10 crash window: the identity is bound before anything is written; a retry after the
// aborted commit finishes the resolution exactly once.
#[test]
fn an_aborted_resolution_is_finished_exactly_once_by_the_same_identity() {
    let f = Fixture::new();
    f.update("a", 10, &["--name", "remote name"]);
    f.update("b", 10, &["--name", "local name"]);
    f.publish("a");
    let accepted = f.head("published");
    let draft = f.head("b");
    let (code, _) = f.rebase("b", 500, &accepted, &draft);
    assert_eq!(code, 2);
    let discarded = f.op_at("b", draft.sequence());
    let mut fired = false;
    let error = match resolve_workspace(
        &store(&f.path("b")),
        &f.path("b"),
        &ResolveRequest {
            op_id: OpId::parse(&id(600)).unwrap(),
            actor: Uuid7::parse(&id(3)).unwrap(),
            discarded: OpId::parse(&discarded).unwrap(),
            expected_draft: draft.revision().clone(),
        },
        &mut |point| {
            if point == CommitPoint::Checked && !fired {
                fired = true;
                Err(Injected)
            } else {
                Ok(())
            }
        },
    ) {
        Ok(_) => panic!("the injected commit fault was not delivered"),
        Err(error) => error,
    };
    assert_eq!(error.code, ErrorCode::Cancelled);
    assert!(fired);
    assert_eq!(f.head("b").manifest_digest(), draft.manifest_digest());
    let (code, resolved) = f.resolve("b", 600, &discarded, &draft);
    assert_eq!(code, 0, "{resolved:?}");
    assert_eq!(text(&resolved, "outcome"), "workspace_resolved");
    let head = f.head("b");
    assert_eq!(head.sequence(), draft.sequence() + 1);
    assert_eq!(head.revision().as_str(), id(600));
    // The claim replays: the retry after the answer changes nothing.
    let (code, replayed) = f.resolve("b", 600, &discarded, &draft);
    assert_eq!(code, 0);
    assert_eq!(text(&replayed, "outcome"), "workspace_resolve_replayed");
    assert_eq!(f.head("b").manifest_digest(), head.manifest_digest());
}

// Mid-interval renunciation works, and unsuitable targets are refused with typed codes.
#[test]
fn mid_interval_renunciation_works_and_unsuitable_targets_are_refused() {
    let f = Fixture::new();
    f.update("b", 10, &["--name", "first"]);
    f.update("b", 11, &["--name", "second"]);
    let draft = f.head("b");
    let first = f.op_at("b", draft.sequence() - 1);
    let (code, resolved) = f.resolve("b", 600, &first, &draft);
    assert_eq!(code, 0, "{resolved:?}");
    assert_eq!(
        resolved
            .get("renounced_operations")
            .unwrap()
            .array()
            .unwrap()
            .len(),
        2,
        "the named operation and the later one are renounced"
    );
    let draft_report = plate::inspect(&store(&f.path("b"))).unwrap();
    let published_report = plate::inspect(&store(&f.path("published"))).unwrap();
    assert_eq!(name_of(&draft_report, 10), name_of(&published_report, 10));
    assert_eq!(name_of(&draft_report, 11), name_of(&published_report, 11));

    // A rebase candidate is administrative work, not an editable command.
    f.update("a", 10, &["--name", "remote"]);
    f.publish("a");
    let accepted = f.head("published");
    let restored = f.head("b");
    let (code, rebased) = f.rebase("b", 610, &accepted, &restored);
    assert_eq!(code, 0, "{rebased:?}");
    let candidate_op = f.op_at("b", f.head("b").sequence());
    let before = f.head("b").manifest_digest();
    let (code, refused) = f.resolve("b", 611, &candidate_op, &f.head("b"));
    assert_eq!(code, 2);
    assert_eq!(text(&refused, "code"), "INVALID_ARGUMENT");
    assert_eq!(f.head("b").manifest_digest(), before);

    // A legacy revision without command evidence inside the renounced range is refused up
    // front: recording the resolution would look like a fix while the interval stays
    // unverifiable.
    let g = Fixture::new();
    g.update("b", 10, &["--name", "temporary"]);
    let edit_sequence = g.head("b").sequence();
    commit_without_evidence(&g.path("b"), &id(604), &id(605));
    let legacy = g.head("b");
    let edit_op = g.op_at("b", edit_sequence);
    let (code, refused) = g.resolve("b", 620, &edit_op, &legacy);
    assert_eq!(code, 2, "{refused:?}");
    assert_eq!(text(&refused, "code"), "INVALID_ARGUMENT");
    assert_eq!(g.head("b").manifest_digest(), legacy.manifest_digest());
    // The incomplete interval stays guarded downstream as well.
    g.update("a", 11, &["--name", "remote"]);
    g.publish("a");
    let g_accepted = g.head("published");
    let (code, result) = g.rebase("b", 622, &g_accepted, &g.head("b"));
    assert_eq!(code, 2, "{result:?}");
    assert_eq!(text(&result, "code"), "DEPENDENCY_UNAVAILABLE");

    // A stale expected head loses the draft CAS with BASE_REVISION_MISMATCH.
    let h = Fixture::new();
    h.update("b", 10, &["--name", "one"]);
    let stale = h.head("b");
    h.update("b", 10, &["--name", "two"]);
    let (code, refused) = h.resolve("b", 630, &h.op_at("b", stale.sequence()), &stale);
    assert_eq!(code, 2);
    assert_eq!(text(&refused, "code"), "BASE_REVISION_MISMATCH");
    // A foreign actor is refused before anything is bound.
    let (code, denied) = h.run(&[
        "resolve-workspace",
        "--workspace-dir",
        h.s("b"),
        "--op-id",
        &id(631),
        "--discard",
        &h.op_at("b", h.head("b").sequence()),
        "--expected-draft",
        h.head("b").revision().as_str(),
        "--actor-id",
        &id(99),
    ]);
    assert_eq!(code, 2);
    assert_eq!(text(&denied, "code"), "PERMISSION_DENIED");
    assert!(
        store(&h.path("b"))
            .changeset_claim(&Uuid7::parse(&id(631)).unwrap())
            .unwrap()
            .is_none()
    );
}

// C04 follow-up: a resolution revision is an undo boundary. Neither direction crosses it:
// undoing or redoing across the boundary would restore the renounced layers while the receipt
// keeps excluding that work, which a later rebase could only refuse as an integrity error.
#[test]
fn a_resolution_revision_is_an_undo_boundary() {
    let f = Fixture::new();
    let fork_name = name_of(&plate::inspect(&store(&f.path("b"))).unwrap(), 10);
    f.update("b", 10, &["--name", "first"]);
    f.update("b", 11, &["--name", "second"]);
    let draft = f.head("b");
    let first = f.op_at("b", draft.sequence() - 1);
    let (code, resolved) = f.resolve("b", 600, &first, &draft);
    assert_eq!(code, 0, "{resolved:?}");
    let boundary = f.head("b");
    let (code, refused) = f.run(&["undo-workspace", "--workspace-dir", f.s("b")]);
    assert_eq!(code, 2, "{refused:?}");
    assert_eq!(text(&refused, "code"), "INVALID_ARGUMENT");
    let (code, refused) = f.run(&["redo-workspace", "--workspace-dir", f.s("b")]);
    assert_eq!(code, 2, "{refused:?}");
    assert_eq!(text(&refused, "code"), "INVALID_ARGUMENT");
    assert_eq!(f.head("b").manifest_digest(), boundary.manifest_digest());

    // Edits after the boundary stay undoable, and their undo stops at the boundary with the
    // renounced work still gone.
    f.update("b", 10, &["--name", "after"]);
    let (code, undone) = f.run(&["undo-workspace", "--workspace-dir", f.s("b")]);
    assert_eq!(code, 0, "{undone:?}");
    assert_eq!(text(&undone, "outcome"), "workspace_undone");
    let report = plate::inspect(&store(&f.path("b"))).unwrap();
    assert_eq!(
        name_of(&report, 10),
        fork_name,
        "no renounced value returned"
    );
    assert_eq!(name_of(&report, 11), fork_name);
    let undone_head = f.head("b");
    let (code, refused) = f.run(&["undo-workspace", "--workspace-dir", f.s("b")]);
    assert_eq!(code, 2, "{refused:?}");
    assert_eq!(text(&refused, "code"), "INVALID_ARGUMENT");
    assert_eq!(f.head("b").manifest_digest(), undone_head.manifest_digest());

    // A pending redo cannot resurrect the renounced work either: the resolution clears the
    // redo stack with the boundary.
    let g = Fixture::new();
    let g_fork = name_of(&plate::inspect(&store(&g.path("b"))).unwrap(), 10);
    g.update("b", 10, &["--name", "first"]);
    g.ok(&["undo-workspace", "--workspace-dir", g.s("b")]);
    let undone = g.head("b");
    let first = g.op_at("b", undone.sequence() - 1);
    let (code, resolved) = g.resolve("b", 610, &first, &undone);
    assert_eq!(code, 0, "{resolved:?}");
    let (code, refused) = g.run(&["redo-workspace", "--workspace-dir", g.s("b")]);
    assert_eq!(code, 2, "{refused:?}");
    assert_eq!(text(&refused, "code"), "INVALID_ARGUMENT");
    let report = plate::inspect(&store(&g.path("b"))).unwrap();
    assert_eq!(name_of(&report, 10), g_fork);
}

// C04 follow-up: undo and redo revisions carry the moved unit's footprint, so a renounced
// range across undo noise stays complete: the resolve succeeds, the rebase plans, and the
// renounced work never reappears.
#[test]
fn a_renounced_range_across_undo_and_redo_stays_verifiable() {
    let f = Fixture::new();
    f.update("a", 10, &["--name", "remote name"]);
    f.update("b", 10, &["--name", "local name"]);
    f.ok(&["undo-workspace", "--workspace-dir", f.s("b")]);
    f.ok(&["redo-workspace", "--workspace-dir", f.s("b")]);
    f.publish("a");
    let accepted = f.head("published");
    let draft = f.head("b");
    // The inherited footprints participate in planning: the conflict is typed, not an
    // unavailable-interval refusal.
    let (code, result) = f.rebase("b", 640, &accepted, &draft);
    assert_eq!(code, 2, "{result:?}");
    assert_eq!(text(&result, "code"), "DEPENDENCY_CONFLICT");
    let discarded = f.op_at("b", draft.sequence() - 2);
    let (code, resolved) = f.resolve("b", 641, &discarded, &draft);
    assert_eq!(code, 0, "{resolved:?}");
    assert_eq!(
        resolved
            .get("renounced_operations")
            .unwrap()
            .array()
            .unwrap()
            .len(),
        3,
        "the edit and both history steps are renounced"
    );
    let resolved_head = f.head("b");
    let (code, rebased) = f.rebase("b", 642, &accepted, &resolved_head);
    assert_eq!(code, 0, "{rebased:?}");
    assert_eq!(text(&rebased, "outcome"), "workspace_rebased");
    let report = plate::inspect(&store(&f.path("b"))).unwrap();
    assert_eq!(
        name_of(&report, 10),
        "remote name",
        "the merged draft holds the accepted state"
    );
}
