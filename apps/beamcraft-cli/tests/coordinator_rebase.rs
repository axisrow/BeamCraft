// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, workspaces and tokens, MPL-2.0.

//! C03-R01..C03-R08: the rebase of an unsent draft through the authenticated coordinator
//! (`docs/design/c03-c06-acceptance.md`). A real `serve-coordinator` process with real token
//! files runs the local rebase semantics on the wire: the token actor is the only actor, the
//! published head never moves by a rebase, the typed result matches the local
//! `rebase-workspace` command, one request identity replays across entry points and crash
//! windows, and bounded refusals write nothing.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use beamcraft_host::revision::RevisionStore;
use beamcraft_host::storage::BlobStore;
use beamcraft_json::{Json, Limits};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::support;

use support::{at, text};

const USER_A: &str = "019a7bf0-0000-7000-8000-00000000ca01";
const USER_B: &str = "019a7bf0-0000-7000-8000-00000000ca02";
const USER_C: &str = "019a7bf0-0000-7000-8000-00000000ca03";
const PLATE: &str = "019a7bf0-0000-7000-8000-000000000310";
const TOKEN_A: &str = "c03r-token-a-0123456789abcdefghijklmnopqrstuv";
const TOKEN_B: &str = "c03r-token-b-0123456789abcdefghijklmnopqrstuv";
const OTHER_TOKEN: &str = "c03r-token-x-0123456789abcdefghijklmnopqrstuv";

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
            shared: support::StoreFixture::new("c03-rebase-tests"),
        };
        fs::create_dir(fixture.shared.root.join("published")).unwrap();
        fs::create_dir(fixture.shared.root.join("workspaces")).unwrap();
        let store = fixture.path("published");
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
            PLATE,
            "--name",
            "Seed",
        ]);
        for (name, user) in [("a", USER_A), ("b", USER_B)] {
            fixture.ok(&[
                "create-workspace",
                "--store",
                &store,
                "--workspace-dir",
                &fixture.path(&format!("workspaces/{name}")),
                "--user-id",
                user,
            ]);
        }
        fixture
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

    /// A token file readable by its owner only.
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

    /// The published head's revision id.
    fn head(&self) -> String {
        self.head_of(&self.path("published"))
    }

    fn head_of(&self, store: &str) -> String {
        text(
            &self.ok(&["open-revision", "--store", store]),
            "revision_id",
        )
    }

    fn edit(&self, workspace: &str, patch: &[&str]) {
        let dir = self.path(&format!("workspaces/{workspace}"));
        let mut args = vec!["update-element", "--store", &dir, "--id", PLATE];
        args.extend_from_slice(patch);
        self.ok(&args);
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

    /// The local C03 rebase of one workspace: the other entry point of the same operation.
    fn rebase_local(
        &self,
        workspace: &str,
        op_id: &str,
        expected_head: &str,
        expected_draft: &str,
    ) -> (i32, Json) {
        self.run(&[
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
        ])
    }

    /// Packages the unsent work: (changeset digest, base revision).
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

    /// The first plate of a store: (name, thickness).
    fn plate(&self, store: &str) -> (String, f64) {
        let inspected = self.ok(&["inspect-elements", "--store", store]);
        let plate = &inspected.get("plates").unwrap().array().unwrap()[0];
        let thickness = at(plate, &["dimensions_mm", "thickness"]).float().unwrap();
        (text(plate, "name"), thickness)
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
    /// Waits for the server to stop after its request budget and returns its final JSON.
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

fn wire_rebase(
    f: &Fixture,
    address: &str,
    token_file: &str,
    workspace: &str,
    op_id: &str,
    expected_head: &str,
    expected_draft: &str,
) -> (i32, Json) {
    f.run(&[
        "coordinator-rebase",
        "--address",
        address,
        "--token-file",
        token_file,
        "--workspace",
        workspace,
        "--op-id",
        op_id,
        "--expected-head",
        expected_head,
        "--expected-draft",
        expected_draft,
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

fn frame(json: &Json) -> Vec<u8> {
    let body = json.encode(&Limits::DEFAULT).unwrap();
    let mut bytes = (body.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(&body);
    bytes
}

/// One raw request frame: fixed keys, simple bounded values (tokens, names, UUIDs), so no
/// JSON escaping is needed.
fn request_bytes(token: &str, op: &str, fields: &[(&str, &str)]) -> Vec<u8> {
    let mut body =
        format!("{{\"protocol\":\"beamcraft-coordinator/1\",\"token\":\"{token}\",\"op\":\"{op}\"");
    for (key, value) in fields {
        body.push_str(&format!(",\"{key}\":\"{value}\""));
    }
    body.push('}');
    let mut bytes = (body.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(body.as_bytes());
    bytes
}

/// Sends one raw frame and reads the answer frame, whatever the protocol inside.
fn call_raw(address: &str, bytes: &[u8]) -> Json {
    let mut stream = TcpStream::connect(address.trim_start_matches("http://")).unwrap();
    stream.write_all(bytes).unwrap();
    let mut length = [0u8; 4];
    stream.read_exact(&mut length).unwrap();
    let length = u32::from_be_bytes(length) as usize;
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).unwrap();
    beamcraft_json::decode(&body, &Limits::DEFAULT).unwrap()
}

/// Sends one authenticated request and drops the connection without reading the answer: the
/// client died after the server accepted the request.
fn send_and_drop(address: &str, bytes: &[u8]) {
    let mut stream = TcpStream::connect(address.trim_start_matches("http://")).unwrap();
    stream.write_all(bytes).unwrap();
    let _ = stream.shutdown(std::net::Shutdown::Both);
    drop(stream);
}

/// The number of accepted revisions of a store and their distinct op identities.
fn history(store: &str) -> (u64, Vec<String>) {
    let store = RevisionStore::new(BlobStore::new(store).unwrap());
    let head = store.head().unwrap().unwrap();
    let mut ops = Vec::new();
    for sequence in 1..=head.sequence() {
        ops.push(
            store
                .info_at(sequence)
                .unwrap()
                .manifest()
                .content()
                .op()
                .to_string(),
        );
    }
    ops.sort();
    (head.sequence(), ops)
}

/// C03-R01: the wire rebase runs the local semantics; the typed result and the new draft head
/// travel in the answer, the published head never moves, and an explicit publish is needed to
/// move it. The conflict branch answers the same bounded report as the local command,
/// byte-stable across retries.
#[test]
fn wire_rebase_merges_and_never_moves_the_published_head() {
    let f = Fixture::new();
    f.edit("b", &["--thickness", "12"]);
    f.edit("a", &["--name", "From A"]);
    let head1 = f.publish_local("a"); // A renames the plate.
    let draft = f.head_of(&f.path("workspaces/b"));
    let token_a = f.token("token-a", TOKEN_A);
    let token_b = f.token("token-b", TOKEN_B);
    let server = f.serve(&[(USER_A, &token_a), (USER_B, &token_b)], 4);
    let address = server.address.clone();

    let (code, answer) = wire_rebase(&f, &address, &token_b, "b", &op(1), &head1, &draft);
    assert_eq!(code, 0, "{answer:?}");
    assert_eq!(text(&answer, "outcome"), "workspace_rebased");
    let merged = f.head_of(&f.path("workspaces/b"));
    assert_eq!(text(&answer, "revision_id"), merged);
    assert_eq!(text(&answer, "base_revision"), head1);
    assert_eq!(text(&answer, "retained_operations"), "1");
    // The published head is unchanged and named in the answer for information only.
    assert_eq!(text(&answer, "current_head"), head1);
    assert_eq!(text(&answer, "current_head_status"), "known");
    assert_eq!(f.head(), head1);
    // The merged draft carries both edits: A's accepted rename and B's thickness.
    let (name, thickness) = f.plate(&f.path("workspaces/b"));
    assert_eq!((name.as_str(), thickness), ("From A", 12.0));

    // The client still needs an explicit publish to move the published head.
    let (digest, base) = f.package("b");
    assert_eq!(base, head1);
    let (code, published) = wire_publish(&f, &address, &token_b, "b", &digest, &head1);
    assert_eq!(code, 0, "{published:?}");
    assert_eq!(text(&published, "outcome"), "changeset_applied");
    let head_moved = f.head();
    assert_ne!(head_moved, head1);
    let (name, thickness) = f.plate(&f.path("published"));
    assert_eq!((name.as_str(), thickness), ("From A", 12.0));

    // A semantic conflict on the wire: a third client, whose workspace forked from the head
    // B just published, accepts a thickness edit on the same plate; B's fresh unsent
    // thickness then conflicts on the atomic shape inputs, and no head or draft moves.
    f.edit("b", &["--thickness", "16"]);
    f.ok(&[
        "create-workspace",
        "--store",
        &f.path("published"),
        "--workspace-dir",
        &f.path("workspaces/c"),
        "--user-id",
        USER_C,
    ]);
    f.edit("c", &["--thickness", "18"]);
    let head2 = f.publish_local("c");
    let draft2 = f.head_of(&f.path("workspaces/b"));
    let (code, conflicted) = wire_rebase(&f, &address, &token_b, "b", &op(2), &head2, &draft2);
    assert_eq!(code, 2, "{conflicted:?}");
    assert_eq!(text(&conflicted, "outcome"), "rejected");
    assert_eq!(text(&conflicted, "code"), "DEPENDENCY_CONFLICT");
    let report = at(&conflicted, &["diagnostics"]).array().unwrap();
    assert_eq!(report.len(), 2, "{report:?}");
    for diagnostic in report {
        assert_eq!(text(diagnostic, "kind"), "write_write");
        assert_eq!(text(diagnostic, "target_kind"), "element");
        assert_eq!(text(diagnostic, "target"), PLATE);
    }
    assert_eq!(text(&conflicted, "current_head"), head2);
    assert_eq!(f.head(), head2);
    assert_eq!(f.head_of(&f.path("workspaces/b")), draft2);
    let (name, thickness) = f.plate(&f.path("workspaces/b"));
    assert_eq!((name.as_str(), thickness), ("From A", 16.0));

    // The same request replays byte-stable, and the local command answers the same report.
    let (code, replay) = wire_rebase(&f, &address, &token_b, "b", &op(2), &head2, &draft2);
    assert_eq!(
        (code, text(&replay, "code").as_str()),
        (2, "DEPENDENCY_CONFLICT")
    );
    assert_eq!(
        at(&replay, &["diagnostics"]),
        at(&conflicted, &["diagnostics"])
    );
    let (local_code, local) = f.run(&[
        "rebase-workspace",
        "--store",
        &f.path("published"),
        "--workspace-dir",
        &f.path("workspaces/b"),
        "--op-id",
        &op(2),
        "--expected-head",
        &head2,
        "--expected-draft",
        &draft2,
    ]);
    assert_eq!(local_code, 2);
    assert_eq!(
        at(&local, &["diagnostics"]),
        at(&conflicted, &["diagnostics"])
    );
    assert_eq!(f.head_of(&f.path("workspaces/b")), draft2);

    let (code, stopped) = server.finish();
    assert_eq!(code, 0);
    assert_eq!(text(&stopped, "outcome"), "stopped");
}

/// C03-R02: the token actor is the only actor. A stranger's token cannot rebase a workspace it
/// does not own, a claimed actor field is an unknown field, and a stranger's token cannot
/// publish another actor's changeset. Every refusal writes nothing.
#[test]
fn the_token_actor_is_the_only_actor() {
    let f = Fixture::new();
    f.edit("a", &["--name", "A1"]);
    let head1 = f.publish_local("a"); // A's edit moves the published head past the seed.
    let token_a = f.token("token-a", TOKEN_A);
    let token_b = f.token("token-b", TOKEN_B);
    let server = f.serve(&[(USER_A, &token_a), (USER_B, &token_b)], 4);
    let address = server.address.clone();
    let a_draft = f.head_of(&f.path("workspaces/a"));

    // B's token cannot rebase A's workspace: refused with nothing written anywhere.
    let (code, body) = wire_rebase(&f, &address, &token_b, "a", &op(1), &head1, &a_draft);
    assert_eq!(code, 2, "{body:?}");
    assert_eq!(text(&body, "code"), "PERMISSION_DENIED");
    assert_eq!(f.head(), head1);
    assert_eq!(f.head_of(&f.path("workspaces/a")), a_draft);

    // A claimed actor field is not an allowed rebase field: refused before any state read.
    let bytes = request_bytes(
        TOKEN_B,
        "rebase",
        &[
            ("workspace", "a"),
            ("actor", USER_B),
            ("expected_head", head1.as_str()),
            ("expected_draft", a_draft.as_str()),
        ],
    );
    let body = call_raw(&address, &bytes);
    assert_eq!(text(&body, "code"), "UNSUPPORTED_PROPERTY");
    assert_eq!(f.head(), head1);
    assert_eq!(f.head_of(&f.path("workspaces/a")), a_draft);

    // The token actor also decides publication: B cannot ship A's changeset.
    f.edit("a", &["--name", "A2"]);
    let (digest, base) = f.package("a");
    assert_eq!(base, head1);
    let (code, body) = wire_publish(&f, &address, &token_b, "a", &digest, &head1);
    assert_eq!(code, 2, "{body:?}");
    assert_eq!(text(&body, "code"), "PERMISSION_DENIED");
    assert_eq!(f.head(), head1);
    let (code, body) = wire_publish(&f, &address, &token_a, "a", &digest, &head1);
    assert_eq!(code, 0, "{body:?}");
    assert_eq!(text(&body, "outcome"), "changeset_applied");

    let (code, stopped) = server.finish();
    assert_eq!(code, 0);
    assert_eq!(text(&stopped, "outcome"), "stopped");
}

/// C03-R03: authentication happens before any state is read. An unknown token, a token file
/// below the length bound and a group-readable token file are refused, and neither the
/// published head nor the draft moved.
#[test]
fn authentication_precedes_any_state_read() {
    let f = Fixture::new();
    f.edit("a", &["--name", "A1"]);
    let head1 = f.publish_local("a");
    let token_a = f.token("token-a", TOKEN_A);
    let wrong = f.token("wrong", OTHER_TOKEN);
    let short = f.token("short", "too-short");
    let server = f.serve(&[(USER_A, &token_a)], 1);
    let address = server.address.clone();
    let draft = f.head_of(&f.path("workspaces/b"));

    let (code, body) = wire_rebase(&f, &address, &wrong, "b", &op(1), &head1, &draft);
    assert_eq!(code, 2, "{body:?}");
    assert_eq!(text(&body, "code"), "PERMISSION_DENIED");
    let (code, body) = wire_rebase(&f, &address, &short, "b", &op(1), &head1, &draft);
    assert_eq!(code, 2, "{body:?}");
    assert_eq!(text(&body, "code"), "INVALID_ARGUMENT");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let open = f.token("open", TOKEN_B);
        fs::set_permissions(&open, fs::Permissions::from_mode(0o644)).unwrap();
        let (code, body) = wire_rebase(&f, &address, &open, "b", &op(1), &head1, &draft);
        assert_eq!(code, 2, "{body:?}");
        assert_eq!(text(&body, "code"), "PERMISSION_DENIED");
    }
    assert_eq!(f.head(), head1);
    assert_eq!(f.head_of(&f.path("workspaces/b")), draft);
    // The wrong-token request is refused at authentication and never counted as handled, so
    // the server does not reach its budget: it is stopped by dropping it.
    drop(server);
}

/// C03-R04: interleaved publication. After B's rebase a third publication moves the head; B's
/// stale publish is refused with the current head, and rebase, repackage and publish succeed in
/// order. The accepted history holds every edit exactly once.
#[test]
fn interleaved_publication_keeps_every_accepted_edit_exactly_once() {
    let f = Fixture::new();
    f.edit("b", &["--thickness", "12"]);
    f.edit("a", &["--name", "A1"]);
    let head1 = f.publish_local("a"); // A renames.
    let token_a = f.token("token-a", TOKEN_A);
    let token_b = f.token("token-b", TOKEN_B);
    let server = f.serve(&[(USER_A, &token_a), (USER_B, &token_b)], 4);
    let address = server.address.clone();
    let draft1 = f.head_of(&f.path("workspaces/b"));

    let (code, answer) = wire_rebase(&f, &address, &token_b, "b", &op(1), &head1, &draft1);
    assert_eq!(code, 0, "{answer:?}");
    let rebased1 = text(&answer, "revision_id");

    // A third publication moves the head after B rebased but before B published.
    let head2 = {
        f.edit("a", &["--name", "A2"]);
        f.publish_local("a")
    };
    let (digest, base) = f.package("b");
    assert_eq!(base, head1);
    let (code, stale) = wire_publish(&f, &address, &token_b, "b", &digest, &head1);
    assert_eq!(code, 2, "{stale:?}");
    assert_eq!(text(&stale, "outcome"), "rejected");
    assert_eq!(text(&stale, "code"), "BASE_REVISION_MISMATCH");
    assert_eq!(text(&stale, "current_head"), head2);
    assert_eq!(f.head(), head2);

    // The only forward path: rebase again, repackage under a new identity, publish.
    let (code, answer) = wire_rebase(&f, &address, &token_b, "b", &op(2), &head2, &rebased1);
    assert_eq!(code, 0, "{answer:?}");
    assert_eq!(text(&answer, "outcome"), "workspace_rebased");
    let (digest2, base2) = f.package("b");
    assert_eq!(base2, head2);
    let (code, published) = wire_publish(&f, &address, &token_b, "b", &digest2, &head2);
    assert_eq!(code, 0, "{published:?}");
    assert_eq!(text(&published, "outcome"), "changeset_applied");

    // The store holds the seed and every edit exactly once, in accepted order.
    let (sequence, ops) = history(&f.path("published"));
    assert_eq!(sequence, 4);
    assert_eq!(ops.len(), 4, "{ops:?}");
    let (name, thickness) = f.plate(&f.path("published"));
    assert_eq!((name.as_str(), thickness), ("A2", 12.0));

    let (code, stopped) = server.finish();
    assert_eq!(code, 0);
    assert_eq!(text(&stopped, "outcome"), "stopped");
}

/// C03-R05: concurrent rebases of one workspace. The same identity admits exactly one merge
/// and replays for the loser; distinct identities admit one merge and refuse the other with a
/// typed mismatch. Two merged draft heads never exist.
#[test]
fn concurrent_rebases_of_one_workspace_admit_exactly_one_merge() {
    let f = Fixture::new();
    f.edit("b", &["--thickness", "12"]);
    f.edit("a", &["--name", "A1"]);
    let head1 = f.publish_local("a");
    let token_a = f.token("token-a", TOKEN_A);
    let token_b = f.token("token-b", TOKEN_B);
    let server = f.serve(&[(USER_A, &token_a), (USER_B, &token_b)], 4);
    let address = server.address.clone();
    let draft = f.head_of(&f.path("workspaces/b"));

    // One request identity, two concurrent clients: one merge, one replay.
    let results: Vec<(String, (i32, Json))> = thread::scope(|s| {
        let handles: Vec<_> = [op(1), op(1)]
            .iter()
            .map(|identity| {
                let (f, address, token_b, head1, draft) = (&f, &address, &token_b, &head1, &draft);
                let identity = identity.clone();
                s.spawn(move || {
                    (
                        identity.clone(),
                        wire_rebase(f, address, token_b, "b", &identity, head1, draft),
                    )
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let rebased = results
        .iter()
        .filter(|(_, (_, body))| text(body, "outcome") == "workspace_rebased")
        .collect::<Vec<_>>();
    let replayed = results
        .iter()
        .filter(|(_, (_, body))| text(body, "outcome") == "workspace_rebase_replayed")
        .collect::<Vec<_>>();
    assert_eq!((rebased.len(), replayed.len()), (1, 1), "{results:?}");
    let merged = text(&rebased[0].1.1, "revision_id");
    assert_eq!(text(&replayed[0].1.1, "revision_id"), merged);
    assert_eq!(f.head_of(&f.path("workspaces/b")), merged);
    assert_eq!(f.head(), head1);
    // The draft store holds the fork, the thickness edit and exactly one merge.
    let before = RevisionStore::new(BlobStore::new(f.path("workspaces/b")).unwrap())
        .head()
        .unwrap()
        .unwrap()
        .sequence();
    assert_eq!(before, 3);

    // Distinct identities against one draft: one merge, one typed mismatch, still one head.
    f.edit("a", &["--name", "A2"]);
    let head2 = f.publish_local("a");
    let current = f.head_of(&f.path("workspaces/b"));
    let results: Vec<(String, (i32, Json))> = thread::scope(|s| {
        let handles: Vec<_> = [op(2), op(3)]
            .iter()
            .map(|identity| {
                let (f, address, token_b, head2, current) =
                    (&f, &address, &token_b, &head2, &current);
                let identity = identity.clone();
                s.spawn(move || {
                    (
                        identity.clone(),
                        wire_rebase(f, address, token_b, "b", &identity, head2, current),
                    )
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let merged: Vec<_> = results.iter().filter(|(_, (code, _))| *code == 0).collect();
    let refused: Vec<_> = results
        .iter()
        .filter(|(_, (code, body))| *code == 2 && text(body, "code") == "BASE_REVISION_MISMATCH")
        .collect();
    assert_eq!((merged.len(), refused.len()), (1, 1), "{results:?}");
    assert_eq!(text(&merged[0].1.1, "outcome"), "workspace_rebased");
    let draft_head = f.head_of(&f.path("workspaces/b"));
    assert_eq!(text(&merged[0].1.1, "revision_id"), draft_head);
    // Exactly one new draft revision: the loser never merged.
    let b_store = RevisionStore::new(BlobStore::new(f.path("workspaces/b")).unwrap());
    assert_eq!(b_store.head().unwrap().unwrap().sequence(), before + 1);

    let (code, stopped) = server.finish();
    assert_eq!(code, 0);
    assert_eq!(text(&stopped, "outcome"), "stopped");
}

/// C03-R06: crash windows. A client killed after the server accepted the request replays its
/// recorded outcome on retry; a server killed while a rebase is in flight leaves a complete
/// state, and the same identity finishes the merge exactly once with no lost unsent edit.
#[test]
fn lost_answers_and_a_killed_server_replay_exactly_once() {
    let f = Fixture::new();
    f.edit("b", &["--thickness", "12"]);
    f.edit("a", &["--name", "From A"]);
    let head1 = f.publish_local("a");
    let token_a = f.token("token-a", TOKEN_A);
    let token_b = f.token("token-b", TOKEN_B);
    let draft = f.head_of(&f.path("workspaces/b"));

    // The client dies before the answer: the merge still happened exactly once.
    let server = f.serve(&[(USER_A, &token_a), (USER_B, &token_b)], 2);
    let address = server.address.clone();
    let identity = op(1);
    let fields = [
        ("workspace", "b"),
        ("op_id", identity.as_str()),
        ("expected_head", head1.as_str()),
        ("expected_draft", draft.as_str()),
    ];
    send_and_drop(&address, &request_bytes(TOKEN_B, "rebase", &fields));
    // The server answers each connection on its own thread, so the retry could overtake the
    // dropped request and do the merge itself. Wait until the dropped request has merged.
    let deadline = Instant::now() + Duration::from_secs(30);
    while f.head_of(&f.path("workspaces/b")) == draft {
        assert!(
            Instant::now() < deadline,
            "the dropped request never merged"
        );
        thread::sleep(Duration::from_millis(20));
    }
    let (code, answer) = wire_rebase(&f, &address, &token_b, "b", &op(1), &head1, &draft);
    assert_eq!(code, 0, "{answer:?}");
    assert_eq!(text(&answer, "outcome"), "workspace_rebase_replayed");
    let merged = text(&answer, "revision_id");
    assert_eq!(f.head_of(&f.path("workspaces/b")), merged);
    assert_eq!(f.head(), head1);
    let (name, thickness) = f.plate(&f.path("workspaces/b"));
    assert_eq!((name.as_str(), thickness), ("From A", 12.0));
    let (code, stopped) = server.finish();
    assert_eq!(code, 0);
    assert_eq!(at(&stopped, &["handled"]), &Json::Number("2".into()));

    // The server is killed while a rebase request is in flight.
    f.edit("b", &["--thickness", "16"]);
    let head2 = {
        f.edit("a", &["--name", "A2"]);
        f.publish_local("a")
    };
    let draft2 = f.head_of(&f.path("workspaces/b"));
    let server = f.serve(&[(USER_A, &token_a), (USER_B, &token_b)], 8);
    let address = server.address.clone();
    thread::scope(|s| {
        s.spawn(|| wire_rebase(&f, &address, &token_b, "b", &op(2), &head2, &draft2));
        thread::sleep(Duration::from_millis(5));
        drop(server); // kill -9 and wait while the request is in flight
    });

    // Restart and finish the same identity: a complete draft either way, merged once.
    let server = f.serve(&[(USER_A, &token_a), (USER_B, &token_b)], 8);
    let address = server.address.clone();
    let mut answer = None;
    for _ in 0..3 {
        let (code, body) = wire_rebase(&f, &address, &token_b, "b", &op(2), &head2, &draft2);
        if code == 0 {
            answer = Some(body);
            break;
        }
        // A typed refusal mid-recovery is bounded; the identity still finishes the merge.
        assert_eq!(text(&body, "outcome"), "rejected", "{body:?}");
    }
    let answer = answer.unwrap_or_else(|| panic!("the rebase never completed"));
    let outcome = text(&answer, "outcome");
    assert!(
        outcome == "workspace_rebased" || outcome == "workspace_rebase_replayed",
        "{answer:?}"
    );
    let merged2 = text(&answer, "revision_id");
    assert_eq!(f.head_of(&f.path("workspaces/b")), merged2);
    assert_eq!(f.head(), head2);
    // Nothing was lost: the merged draft carries A's accepted name and B's thickness once.
    let (name, thickness) = f.plate(&f.path("workspaces/b"));
    assert_eq!((name.as_str(), thickness), ("A2", 16.0));
    let (code, replay) = wire_rebase(&f, &address, &token_b, "b", &op(2), &head2, &draft2);
    assert_eq!(code, 0, "{replay:?}");
    assert_eq!(text(&replay, "outcome"), "workspace_rebase_replayed");
    assert_eq!(text(&replay, "revision_id"), merged2);
    drop(server);
}

/// C03-R07: bounded wire behaviour. An unknown operation, an unknown field, an oversized frame
/// and a wrong protocol are typed refusals that write nothing.
#[test]
fn bounded_wire_refusals_write_nothing() {
    let f = Fixture::new();
    f.edit("a", &["--name", "A1"]);
    let head1 = f.publish_local("a");
    let token_a = f.token("token-a", TOKEN_A);
    let token_b = f.token("token-b", TOKEN_B);
    let server = f.serve(&[(USER_A, &token_a), (USER_B, &token_b)], 2);
    let address = server.address.clone();
    let draft = f.head_of(&f.path("workspaces/b"));

    // The transport-level refusals first: they are never counted as handled, and the two
    // handler-level ones below exhaust the server's budget.
    assert_eq!(
        text(&call_raw(&address, &[0xff, 0xff, 0xff, 0xff]), "code"),
        "LIMIT_EXCEEDED"
    );
    let wrong = frame(&Json::object([
        ("protocol", Json::text("beamcraft-coordinator/0")),
        ("token", Json::text(TOKEN_B)),
        ("op", Json::text("rebase")),
    ]));
    assert_eq!(
        text(&call_raw(&address, &wrong), "code"),
        "UNSUPPORTED_VERSION"
    );
    let unknown_op = frame(&Json::object([
        ("protocol", Json::text("beamcraft-coordinator/1")),
        ("token", Json::text(TOKEN_B)),
        ("op", Json::text("rebase-x")),
    ]));
    assert_eq!(
        text(&call_raw(&address, &unknown_op), "code"),
        "UNSUPPORTED_COMMAND"
    );
    let extra = request_bytes(
        TOKEN_B,
        "rebase",
        &[
            ("workspace", "b"),
            ("op_id", &op(1)),
            ("expected_head", &head1),
            ("expected_draft", &draft),
            ("unexpected", "value"),
        ],
    );
    assert_eq!(
        text(&call_raw(&address, &extra), "code"),
        "UNSUPPORTED_PROPERTY"
    );

    assert_eq!(f.head(), head1);
    assert_eq!(f.head_of(&f.path("workspaces/b")), draft);
    // Only the two handler-level refusals count as handled; the transport-level frame
    // refusals never reach the queue.
    let (code, stopped) = server.finish();
    assert_eq!(code, 0);
    assert_eq!(at(&stopped, &["handled"]), &Json::Number("2".into()));
}

/// C03-R08: one request identity submitted through the local CLI and then through the
/// coordinator returns the recorded result; a second merge under that identity never happens.
#[test]
fn one_identity_replays_across_the_local_cli_and_the_wire() {
    let f = Fixture::new();
    f.edit("b", &["--thickness", "12"]);
    f.edit("a", &["--name", "A1"]);
    let head1 = f.publish_local("a");
    let draft = f.head_of(&f.path("workspaces/b"));
    let token_b = f.token("token-b", TOKEN_B);

    let (code, local) = f.rebase_local("b", &op(1), &head1, &draft);
    assert_eq!(code, 0, "{local:?}");
    assert_eq!(text(&local, "outcome"), "workspace_rebased");
    let merged = at(&local, &["revision", "revision_id"])
        .string()
        .unwrap()
        .to_owned();

    let server = f.serve(&[(USER_B, &token_b)], 1);
    let address = server.address.clone();
    let (code, answer) = wire_rebase(&f, &address, &token_b, "b", &op(1), &head1, &draft);
    assert_eq!(code, 0, "{answer:?}");
    assert_eq!(text(&answer, "outcome"), "workspace_rebase_replayed");
    assert_eq!(text(&answer, "revision_id"), merged);
    assert_eq!(f.head_of(&f.path("workspaces/b")), merged);
    assert_eq!(f.head(), head1);
    let (code, stopped) = server.finish();
    assert_eq!(code, 0);
    assert_eq!(at(&stopped, &["handled"]), &Json::Number("1".into()));
}
