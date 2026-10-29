// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, workspaces and tokens, MPL-2.0.

//! C06-01..C06-07: compensation of a published operation as a new forward operation
//! (`docs/design/c03-c06-acceptance.md`). A compensation appends one evidence-bearing draft
//! operation that restores the target's own effect, refuses by name when any accepted
//! operation after the target intersects its footprint, replays exactly once across entry
//! points and crash windows, and never rolls back another actor's later edit.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use beamcraft_cli::workspace::compensation::{CompensationRequest, compensate_workspace};
use beamcraft_host::revision::{CommitPoint, RevisionStore, Uuid7};
use beamcraft_host::storage::BlobStore;
use beamcraft_json::{Json, Limits};
use beamcraft_model::id::OpId;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::panic::{AssertUnwindSafe, resume_unwind};
use std::process::{Child, ChildStdout, Command, Stdio};

use crate::support;

use support::{at, text};

const USER_A: &str = "019a7bf0-0000-7000-8000-00000000cb01";
const USER_B: &str = "019a7bf0-0000-7000-8000-00000000cb02";
const PLATE: &str = "019a7bf0-0000-7000-8000-000000000310";
const SPARE: &str = "019a7bf0-0000-7000-8000-000000000320";
const TOKEN_A: &str = "c06-token-a-0123456789abcdefghijklmnopqrstuv";
const TOKEN_B: &str = "c06-token-b-0123456789abcdefghijklmnopqrstuv";

/// Distinct bounded request identities.
fn op(n: usize) -> String {
    format!("019a7bf0-0000-7000-8000-0000000c{n:04x}")
}

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("c06-compensation-tests"),
        };
        fs::create_dir(fixture.shared.root.join("published")).unwrap();
        fs::create_dir(fixture.shared.root.join("workspaces")).unwrap();
        let store = fixture.path("published");
        for (id, name) in [(PLATE, "Seed"), (SPARE, "Spare")] {
            fixture.ok(&[
                "create-plate",
                "--store",
                &store,
                "--width",
                "1000",
                "--length",
                "500",
                "--thickness",
                "10",
                "--new-id",
                id,
                "--name",
                name,
            ]);
        }
        fixture.workspace("a", USER_A);
        fixture
    }

    /// A workspace forked from the current accepted head, on demand.
    fn workspace(&self, name: &str, user: &str) {
        self.ok(&[
            "create-workspace",
            "--store",
            &self.path("published"),
            "--workspace-dir",
            &self.path(&format!("workspaces/{name}")),
            "--user-id",
            user,
        ]);
    }

    fn path(&self, name: &str) -> String {
        self.shared.root.join(name).to_str().unwrap().to_owned()
    }

    fn run(&self, args: &[&str]) -> (i32, Json) {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .output()
            .unwrap();
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
        (output.status.code().unwrap(), body)
    }

    fn ok(&self, args: &[&str]) -> Json {
        let (code, body) = self.run(args);
        assert_eq!(code, 0, "{args:?}: {body:?}");
        body
    }

    fn token(&self, name: &str, value: &str) -> String {
        let path = self.shared.root.join(name);
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(&path)
            .unwrap()
            .write_all(value.as_bytes())
            .unwrap();
        path.to_str().unwrap().to_owned()
    }

    fn head(&self) -> String {
        self.head_of(&self.path("published"))
    }

    fn head_of(&self, store: &str) -> String {
        text(
            &self.ok(&["open-revision", "--store", store]),
            "revision_id",
        )
    }

    /// The store's own plate facts: (name, thickness) of the plate with `id`.
    fn plate(&self, store: &str, id: &str) -> (String, f64) {
        let inspected = self.ok(&["inspect-elements", "--store", store]);
        let plates = inspected.get("plates").unwrap().array().unwrap();
        for plate in plates {
            if text(plate, "element_id") == id {
                let thickness = at(plate, &["dimensions_mm", "thickness"]).float().unwrap();
                return (text(plate, "name"), thickness);
            }
        }
        panic!("plate {id} is missing from {store}");
    }

    fn plate_count(&self, store: &str) -> usize {
        let inspected = self.ok(&["inspect-elements", "--store", store]);
        inspected.get("plates").unwrap().array().unwrap().len()
    }

    fn edit(&self, workspace: &str, id: &str, patch: &[&str]) {
        let dir = self.path(&format!("workspaces/{workspace}"));
        let mut args = vec!["update-element", "--store", &dir, "--id", id];
        args.extend_from_slice(patch);
        self.ok(&args);
    }

    fn create_plate(&self, workspace: &str, id: &str, name: &str) {
        self.ok(&[
            "create-plate",
            "--store",
            &self.path(&format!("workspaces/{workspace}")),
            "--width",
            "800",
            "--length",
            "400",
            "--thickness",
            "6",
            "--new-id",
            id,
            "--name",
            name,
        ]);
    }

    fn delete(&self, workspace: &str, id: &str) {
        self.ok(&[
            "delete-element",
            "--store",
            &self.path(&format!("workspaces/{workspace}")),
            "--id",
            id,
        ]);
    }

    fn publish_local(&self, workspace: &str) -> String {
        self.ok(&[
            "publish-workspace",
            "--store",
            &self.path("published"),
            "--workspace-dir",
            &self.path(&format!("workspaces/{workspace}")),
        ]);
        self.head()
    }

    /// The local C06 compensation.
    fn compensate(
        &self,
        workspace: &str,
        op_id: &str,
        target: &str,
        expected_head: &str,
    ) -> (i32, Json) {
        self.run(&[
            "compensate-workspace",
            "--store",
            &self.path("published"),
            "--workspace-dir",
            &self.path(&format!("workspaces/{workspace}")),
            "--target-revision",
            target,
            "--op-id",
            op_id,
            "--expected-head",
            expected_head,
        ])
    }

    /// The local C03 rebase, used to move a workspace onto the current accepted head.
    fn rebase(&self, workspace: &str, op_id: &str, expected_head: &str, expected_draft: &str) {
        let (code, body) = self.run(&[
            "rebase-workspace",
            "--store",
            &self.path("published"),
            "--workspace-dir",
            &self.path(&format!("workspaces/{workspace}")),
            "--op-id",
            op_id,
            "--expected-head",
            expected_head,
            "--expected-draft",
            expected_draft,
        ]);
        assert_eq!(code, 0, "{body:?}");
    }

    fn draft_head(&self, workspace: &str) -> String {
        self.head_of(&self.path(&format!("workspaces/{workspace}")))
    }

    fn package(&self, workspace: &str) -> (String, String) {
        let changeset = at(
            &self.ok(&[
                "create-changeset",
                "--store",
                &self.path("published"),
                "--workspace-dir",
                &self.path(&format!("workspaces/{workspace}")),
            ]),
            &["changeset"],
        )
        .clone();
        (
            text(&changeset, "digest"),
            text(&changeset, "base_revision"),
        )
    }

    fn serve(&self, clients: &[(&str, &str)], max_requests: u32) -> Server {
        let (store, root) = (self.path("published"), self.path("workspaces"));
        let max = max_requests.to_string();
        let mut args = vec![
            "serve-coordinator".to_owned(),
            "--store".into(),
            store,
            "--workspaces-root".into(),
            root,
            "--max-requests".into(),
            max,
        ];
        for (actor, file) in clients {
            args.push("--client".into());
            args.push(format!("{actor}:{file}"));
        }
        let mut child = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let listening = beamcraft_json::decode(line.trim_end().as_bytes(), &Limits::DEFAULT)
            .unwrap_or_else(|_| panic!("the coordinator did not announce itself: {line}"));
        assert_eq!(text(&listening, "outcome"), "listening", "{line}");
        Server {
            child,
            stdout,
            address: text(&listening, "address"),
        }
    }
}

struct Server {
    child: Child,
    stdout: BufReader<ChildStdout>,
    address: String,
}

impl Server {
    fn finish(mut self) -> (i32, Json) {
        let mut rest = Vec::new();
        self.stdout.read_to_end(&mut rest).unwrap();
        let code = self.child.wait().unwrap().code().unwrap();
        (
            code,
            beamcraft_json::decode(&rest, &Limits::DEFAULT).unwrap(),
        )
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wire_compensate(
    f: &Fixture,
    address: &str,
    token_file: &str,
    workspace: &str,
    op_id: &str,
    target: &str,
    expected_head: &str,
) -> (i32, Json) {
    f.run(&[
        "coordinator-compensate",
        "--address",
        address,
        "--token-file",
        token_file,
        "--workspace",
        workspace,
        "--op-id",
        op_id,
        "--target-revision",
        target,
        "--expected-head",
        expected_head,
    ])
}

fn wire_publish(
    f: &Fixture,
    address: &str,
    token_file: &str,
    workspace: &str,
    digest: &str,
    expected_head: &str,
) -> (i32, Json) {
    f.run(&[
        "publish-changeset",
        "--address",
        address,
        "--token-file",
        token_file,
        "--workspace",
        workspace,
        "--changeset",
        digest,
        "--expected-head",
        expected_head,
    ])
}

/// The in-process compensation with a crash hook: dies at `point` of the workspace commit.
fn crashed_compensation(
    f: &Fixture,
    workspace: &str,
    op_id: &str,
    target: &str,
    head: &str,
    point: CommitPoint,
) -> bool {
    struct Crash;
    let dir = f.path(&format!("workspaces/{workspace}"));
    let published = RevisionStore::new(BlobStore::new(f.path("published")).unwrap());
    let draft = RevisionStore::new(BlobStore::new(&dir).unwrap());
    let request = CompensationRequest {
        op_id: OpId::parse(op_id).unwrap(),
        actor: Uuid7::parse(USER_A).unwrap(),
        target_revision: Uuid7::parse(target).unwrap(),
        expected_head: Uuid7::parse(head).unwrap(),
    };
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        compensate_workspace(
            &published,
            &draft,
            std::path::Path::new(&dir),
            &request,
            &mut |reached: CommitPoint| {
                if reached == point {
                    resume_unwind(Box::new(Crash));
                }
                Ok(())
            },
        )
    }));
    result.is_err()
}

/// Closes an interrupted workspace commit the way the operator does after a crash (U07).
fn recover(f: &Fixture, store: &str) {
    f.ok(&["recover-revision", "--store", store, "--min-age-secs", "0"]);
}

/// C06-01: a published edit is compensated with a new forward operation; reopening the draft
/// shows the state before the target; the ordinary publication path accepts it; the original
/// operation and the compensation both stay in the accepted history.
#[test]
fn a_published_edit_is_compensated_forward_and_published() {
    let f = Fixture::new();
    f.edit("a", PLATE, &["--thickness", "12"]);
    let head1 = f.publish_local("a");
    let (code, answer) = f.compensate("a", &op(1), &head1, &head1);
    assert_eq!(code, 0, "{answer:?}");
    assert_eq!(text(&answer, "outcome"), "compensated");
    assert_eq!(text(&answer, "target_revision"), head1);
    // The draft holds the state before the target.
    let (name, thickness) = f.plate(&f.path("workspaces/a"), PLATE);
    assert_eq!((name.as_str(), thickness), ("Seed", 10.0));
    // The compensation is an ordinary unsent operation: package and publish it.
    let (digest, base) = f.package("a");
    assert_eq!(base, head1);
    let token_a = f.token("token-a", TOKEN_A);
    let server = f.serve(&[(USER_A, &token_a)], 1);
    let (code, published) = wire_publish(&f, &server.address, &token_a, "a", &digest, &head1);
    assert_eq!(code, 0, "{published:?}");
    assert_eq!(text(&published, "outcome"), "changeset_applied");
    let head2 = f.head();
    assert_ne!(head2, head1);
    let (name, thickness) = f.plate(&f.path("published"), PLATE);
    assert_eq!((name.as_str(), thickness), ("Seed", 10.0));
    // The original and the compensation are both preserved in the accepted history: the two
    // seed plates, the target operation and the compensation publication.
    let store = RevisionStore::new(BlobStore::new(f.path("published")).unwrap());
    assert_eq!(store.head().unwrap().unwrap().sequence(), 4);
    let (code, _) = server.finish();
    assert_eq!(code, 0);
}

/// C06-01/C06-02: a compensation of a published delete restores the element; a later edit by
/// another actor on the same element is never rolled back when it does not intersect.
#[test]
fn a_delete_compensation_restores_the_element() {
    let f = Fixture::new();
    f.delete("a", SPARE);
    let head1 = f.publish_local("a");
    let (code, answer) = f.compensate("a", &op(1), &head1, &head1);
    assert_eq!(code, 0, "{answer:?}");
    assert_eq!(text(&answer, "outcome"), "compensated");
    // The spare plate is back with its authored content.
    let (name, thickness) = f.plate(&f.path("workspaces/a"), SPARE);
    assert_eq!((name.as_str(), thickness), ("Spare", 10.0));
    assert_eq!(f.plate_count(&f.path("workspaces/a")), 2);
    // Publish and check the accepted store.
    let (digest, base) = f.package("a");
    assert_eq!(base, head1);
    let token_a = f.token("token-a", TOKEN_A);
    let server = f.serve(&[(USER_A, &token_a)], 1);
    let (code, published) = wire_publish(&f, &server.address, &token_a, "a", &digest, &head1);
    assert_eq!(code, 0, "{published:?}");
    assert_eq!(f.plate_count(&f.path("published")), 2);
    let (code, _) = server.finish();
    assert_eq!(code, 0);
}

/// C06-02: a later accepted edit of another actor on the same semantic groups refuses the
/// compensation with a typed report naming that operation; nothing is written anywhere.
#[test]
fn a_conflicting_compensation_names_the_later_operation_and_refuses() {
    let f = Fixture::new();
    f.edit("a", PLATE, &["--thickness", "12"]);
    let head1 = f.publish_local("a"); // A's target operation.
    // B forks from A's head, so its later edit on the same groups publishes directly.
    f.workspace("b", USER_B);
    f.edit("b", PLATE, &["--thickness", "20"]);
    let head2 = f.publish_local("b"); // B's later edit on the same groups.
    // A moves its workspace onto B's head with one disjoint unsent plate and publishes it:
    // the compensation requires a clean draft.
    f.create_plate("a", "019a7bf0-0000-7000-8000-000000000330", "From A");
    let draft = f.draft_head("a");
    f.rebase("a", &op(1), &head2, &draft);
    let head3 = f.publish_local("a");
    let (code, answer) = f.compensate("a", &op(2), &head1, &head3);
    assert_eq!(code, 2, "{answer:?}");
    assert_eq!(text(&answer, "code"), "DEPENDENCY_CONFLICT");
    let report = at(&answer, &["diagnostics"]).array().unwrap();
    let named = report
        .iter()
        .find(|d| text(d, "accepted_op") == head2)
        .unwrap_or_else(|| panic!("no diagnostic names B's operation: {answer:?}"));
    assert_eq!(text(named, "kind"), "write_write");
    // Nothing moved: B's value stands, A's published plate is still there.
    let (name, thickness) = f.plate(&f.path("published"), PLATE);
    assert_eq!((name.as_str(), thickness), ("Seed", 20.0));
    assert_eq!(f.head(), head3);
    assert_eq!(f.plate_count(&f.path("workspaces/a")), 3);
}

/// C06-02: a later accepted edit of another actor that does not intersect the target's
/// footprint survives the compensation verbatim, and the accepted order keeps both.
#[test]
fn a_disjoint_later_edit_survives_a_compensation() {
    let f = Fixture::new();
    f.edit("a", PLATE, &["--thickness", "12"]);
    let head1 = f.publish_local("a");
    // B forks from A's head; its rename touches a disjoint semantic group.
    f.workspace("b", USER_B);
    f.edit("b", PLATE, &["--name", "Renamed by B"]);
    let head2 = f.publish_local("b");
    // A moves onto B's head with one unsent plate and publishes it first: the compensation
    // requires a clean draft, so the unsent work goes through the ordinary path, never an
    // implicit drop.
    f.create_plate("a", "019a7bf0-0000-7000-8000-000000000330", "From A");
    let draft = f.draft_head("a");
    f.rebase("a", &op(1), &head2, &draft);
    let head3 = f.publish_local("a");
    let (code, answer) = f.compensate("a", &op(2), &head1, &head3);
    assert_eq!(code, 0, "{answer:?}");
    assert_eq!(text(&answer, "outcome"), "compensated");
    // B's rename and the published plate sit in the draft next to the reversal.
    let (name, thickness) = f.plate(&f.path("workspaces/a"), PLATE);
    assert_eq!((name.as_str(), thickness), ("Renamed by B", 10.0));
    assert_eq!(f.plate_count(&f.path("workspaces/a")), 3);
    // Publishing keeps every edit in the accepted store.
    let (digest, base) = f.package("a");
    assert_eq!(base, head3);
    let token_a = f.token("token-a", TOKEN_A);
    let server = f.serve(&[(USER_A, &token_a)], 1);
    let (code, published) = wire_publish(&f, &server.address, &token_a, "a", &digest, &head3);
    assert_eq!(code, 0, "{published:?}");
    let (name, thickness) = f.plate(&f.path("published"), PLATE);
    assert_eq!((name.as_str(), thickness), ("Renamed by B", 10.0));
    assert_eq!(f.plate_count(&f.path("published")), 3);
    // The two seed plates, A's target, B's rename, A's plate and the compensation.
    let store = RevisionStore::new(BlobStore::new(f.path("published")).unwrap());
    assert_eq!(store.head().unwrap().unwrap().sequence(), 6);
    let (code, _) = server.finish();
    assert_eq!(code, 0);
}

/// A draft with unsent operations refuses the compensation with a typed remedy instead of
/// dropping the unsent work: the candidate would replace the whole draft content.
#[test]
fn a_dirty_draft_refuses_the_compensation_and_keeps_its_unsent_work() {
    let f = Fixture::new();
    f.edit("a", PLATE, &["--thickness", "12"]);
    let head1 = f.publish_local("a");
    f.create_plate("a", "019a7bf0-0000-7000-8000-000000000330", "From A");
    let draft = f.draft_head("a");
    let (code, answer) = f.compensate("a", &op(1), &head1, &head1);
    assert_eq!(code, 2, "{answer:?}");
    assert_eq!(text(&answer, "code"), "INVALID_ARGUMENT");
    // Nothing was written and the unsent plate is still there.
    assert_eq!(f.draft_head("a"), draft);
    assert_eq!(f.plate_count(&f.path("workspaces/a")), 3);
    assert_eq!(f.head(), head1);
    // The published plate keeps flowing: publish the unsent work, then compensate.
    let head2 = f.publish_local("a");
    let (code, answer) = f.compensate("a", &op(2), &head1, &head2);
    assert_eq!(code, 0, "{answer:?}");
    assert_eq!(text(&answer, "outcome"), "compensated");
    let (name, thickness) = f.plate(&f.path("workspaces/a"), PLATE);
    assert_eq!((name.as_str(), thickness), ("Seed", 10.0));
}

/// C06-03/C06-07: a target whose effect later accepted operations already changed refuses and
/// names the operation that now owns the state; a target outside the store is a typed refusal.
#[test]
fn a_superseded_or_unknown_target_refuses_by_name() {
    let f = Fixture::new();
    f.delete("a", SPARE);
    let delete_head = f.publish_local("a");
    // A recreates the same identity: the delete's compensation would now clash with the
    // recreate's lifecycle write, so the report names the recreate operation.
    f.create_plate("a", SPARE, "Spare");
    let head2 = f.publish_local("a");
    let (code, answer) = f.compensate("a", &op(1), &delete_head, &head2);
    assert_eq!(code, 2, "{answer:?}");
    assert_eq!(text(&answer, "code"), "DEPENDENCY_CONFLICT");
    let report = at(&answer, &["diagnostics"]).array().unwrap();
    assert!(!report.is_empty(), "{answer:?}");
    assert_eq!(text(&report[0], "accepted_op"), head2);
    assert_eq!(f.plate_count(&f.path("workspaces/a")), 2);
    // An unknown target is a typed refusal, not a crash and not a silent success.
    let (code, answer) = f.compensate("a", &op(2), "019a7bf0-0000-7000-8000-00000000dead", &head2);
    assert_eq!(code, 2, "{answer:?}");
    assert_eq!(text(&answer, "code"), "INVALID_ARGUMENT");
    // The very first accepted revision has no basis to restore.
    let store = RevisionStore::new(BlobStore::new(f.path("published")).unwrap());
    let first = store.info_at(1).unwrap().revision().as_str().to_owned();
    let (code, answer) = f.compensate("a", &op(3), &first, &head2);
    assert_eq!(code, 2, "{answer:?}");
    assert_eq!(text(&answer, "code"), "INVALID_ARGUMENT");
}

/// C06-04: the same request identity replays the recorded outcome exactly once; another
/// payload under that identity is refused.
#[test]
fn a_compensation_replays_exactly_once() {
    let f = Fixture::new();
    f.edit("a", PLATE, &["--thickness", "12"]);
    let head1 = f.publish_local("a");
    let (code, first) = f.compensate("a", &op(1), &head1, &head1);
    assert_eq!(code, 0, "{first:?}");
    let revision = at(&first, &["revision", "revision_id"])
        .string()
        .unwrap()
        .to_owned();
    // The same identity replays without writing.
    let (code, replay) = f.compensate("a", &op(1), &head1, &head1);
    assert_eq!(code, 0, "{replay:?}");
    assert_eq!(text(&replay, "outcome"), "compensation_replayed");
    assert_eq!(
        at(&replay, &["revision", "revision_id"]).string().unwrap(),
        revision
    );
    let store = RevisionStore::new(BlobStore::new(f.path("workspaces/a")).unwrap());
    assert_eq!(store.head().unwrap().unwrap().sequence(), 3);
    // A different payload under the same identity is reuse.
    f.edit("a", PLATE, &["--thickness", "14"]);
    let head2 = f.publish_local("a");
    let (code, reuse) = f.compensate("a", &op(1), &head2, &head2);
    assert_eq!(code, 2, "{reuse:?}");
    assert_eq!(text(&reuse, "code"), "OP_ID_REUSE");
}

/// C06-05: a kill before the durable switch leaves no half state; the retry commits the
/// compensation exactly once.
#[test]
fn a_compensation_killed_before_the_switch_recovers_exactly_once() {
    let f = Fixture::new();
    f.edit("a", PLATE, &["--thickness", "12"]);
    let head1 = f.publish_local("a");
    assert!(crashed_compensation(
        &f,
        "a",
        &op(1),
        &head1,
        &head1,
        CommitPoint::ManifestStored
    ));
    let dir = f.path("workspaces/a");
    recover(&f, &dir);
    let (code, answer) = f.compensate("a", &op(1), &head1, &head1);
    assert_eq!(code, 0, "{answer:?}");
    let outcome = text(&answer, "outcome");
    assert!(
        outcome == "compensated" || outcome == "compensation_replayed",
        "{answer:?}"
    );
    let (name, thickness) = f.plate(&f.path("workspaces/a"), PLATE);
    assert_eq!((name.as_str(), thickness), ("Seed", 10.0));
    let store = RevisionStore::new(BlobStore::new(&dir).unwrap());
    assert_eq!(store.head().unwrap().unwrap().sequence(), 3);
}

/// C06-05: a kill after the switch but before the answer leaves the committed compensation;
/// recovery and the same identity replay it exactly once.
#[test]
fn a_compensation_killed_after_the_switch_replays_exactly_once() {
    let f = Fixture::new();
    f.edit("a", PLATE, &["--thickness", "12"]);
    let head1 = f.publish_local("a");
    assert!(crashed_compensation(
        &f,
        "a",
        &op(1),
        &head1,
        &head1,
        CommitPoint::Switched
    ));
    let dir = f.path("workspaces/a");
    recover(&f, &dir);
    let (code, answer) = f.compensate("a", &op(1), &head1, &head1);
    assert_eq!(code, 0, "{answer:?}");
    assert_eq!(text(&answer, "outcome"), "compensation_replayed");
    let (name, thickness) = f.plate(&f.path("workspaces/a"), PLATE);
    assert_eq!((name.as_str(), thickness), ("Seed", 10.0));
    let store = RevisionStore::new(BlobStore::new(&dir).unwrap());
    assert_eq!(store.head().unwrap().unwrap().sequence(), 3);
}

/// C06-06: the wire compensation runs with the token actor. A stranger's token is refused
/// with nothing written; the owner's token compensates, and the published head moves only
/// through an explicit publish. One request identity replays across the wire.
#[test]
fn the_wire_compensates_for_the_owner_only() {
    let f = Fixture::new();
    f.edit("a", PLATE, &["--thickness", "12"]);
    let head1 = f.publish_local("a");
    let draft = f.draft_head("a");
    let token_a = f.token("token-a", TOKEN_A);
    let token_b = f.token("token-b", TOKEN_B);
    let server = f.serve(&[(USER_A, &token_a), (USER_B, &token_b)], 3);
    let address = server.address.clone();

    // The stranger's token cannot compensate A's workspace; nothing is written.
    let (code, body) = wire_compensate(&f, &address, &token_b, "a", &op(1), &head1, &head1);
    assert_eq!(code, 2, "{body:?}");
    assert_eq!(text(&body, "code"), "PERMISSION_DENIED");
    assert_eq!(f.head(), head1);
    assert_eq!(f.draft_head("a"), draft);

    // The owner compensates on the wire; the published head is unchanged.
    let (code, answer) = wire_compensate(&f, &address, &token_a, "a", &op(1), &head1, &head1);
    assert_eq!(code, 0, "{answer:?}");
    assert_eq!(text(&answer, "outcome"), "compensated");
    assert_eq!(text(&answer, "target_revision"), head1);
    assert_eq!(text(&answer, "current_head"), head1);
    assert_eq!(f.head(), head1);
    let (name, thickness) = f.plate(&f.path("workspaces/a"), PLATE);
    assert_eq!((name.as_str(), thickness), ("Seed", 10.0));

    // The same identity replays across entry points: the local CLI sees the wire result.
    let (code, replay) = f.compensate("a", &op(1), &head1, &head1);
    assert_eq!(code, 0, "{replay:?}");
    assert_eq!(text(&replay, "outcome"), "compensation_replayed");

    // An explicit publish moves the head.
    let (digest, base) = f.package("a");
    assert_eq!(base, head1);
    let (code, published) = wire_publish(&f, &address, &token_a, "a", &digest, &head1);
    assert_eq!(code, 0, "{published:?}");
    assert_eq!(text(&published, "outcome"), "changeset_applied");
    let (name, thickness) = f.plate(&f.path("published"), PLATE);
    assert_eq!((name.as_str(), thickness), ("Seed", 10.0));

    let (code, stopped) = server.finish();
    assert_eq!(code, 0);
    assert_eq!(text(&stopped, "outcome"), "stopped");
}

/// C06-06: bounded wire behaviour of the new operation — an unknown field is refused before
/// any state is read and writes nothing.
#[test]
fn the_wire_compensation_refuses_unknown_fields() {
    let f = Fixture::new();
    f.edit("a", PLATE, &["--thickness", "12"]);
    let head1 = f.publish_local("a");
    let token_a = f.token("token-a", TOKEN_A);
    let server = f.serve(&[(USER_A, &token_a)], 1);
    let address = server.address.clone();
    let body = format!(
        "{{\"protocol\":\"beamcraft-coordinator/1\",\"token\":\"{TOKEN_A}\",\"op\":\"compensate\",\
         \"workspace\":\"a\",\"op_id\":\"{}\",\"target_revision\":\"{head1}\",\
         \"expected_head\":\"{head1}\",\"extra\":\"value\"}}",
        op(1)
    );
    let mut bytes = (body.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(body.as_bytes());
    let mut stream = TcpStream::connect(address.trim_start_matches("http://")).unwrap();
    stream.write_all(&bytes).unwrap();
    let mut length = [0u8; 4];
    stream.read_exact(&mut length).unwrap();
    let length = u32::from_be_bytes(length) as usize;
    let mut answer = vec![0u8; length];
    stream.read_exact(&mut answer).unwrap();
    let answer = beamcraft_json::decode(&answer, &Limits::DEFAULT).unwrap();
    assert_eq!(text(&answer, "code"), "UNSUPPORTED_PROPERTY");
    assert_eq!(f.head(), head1);
    let (code, stopped) = server.finish();
    assert_eq!(code, 0);
    assert_eq!(at(&stopped, &["handled"]), &Json::Number("1".into()));
}
