// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, workspaces and tokens, MPL-2.0.

//! C02 end to end: a real `serve-coordinator` process and `publish-changeset` /
//! `inspect-coordinator-head` clients. The coordinator does not start without a token; a wrong token is
//! refused; the token decides the actor; two concurrent publications from one base accept exactly
//! one and the accepted head is never overwritten; the same payload replays; workspace names stay
//! inside the workspaces root.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_json::{Json, Limits};
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, ChildStdout, Command, Stdio};

const USER: &str = "019a7bf0-0000-7000-8000-00000000c201";
const STRANGER: &str = "019a7bf0-0000-7000-8000-00000000c202";
const PLATE: &str = "019a7bf0-0000-7000-8000-000000000210";
const PLATE2: &str = "019a7bf0-0000-7000-8000-000000000211";
/// Two more actors for the C03 rebase tests; each workspace is owned by one of them.
const USER_B: &str = "019a7bf0-0000-7000-8000-00000000c203";
const USER_C: &str = "019a7bf0-0000-7000-8000-00000000c204";
const TOKEN_C: &str = "c03-third-token-0123456789abcdefghijklmno";
const TOKEN_STRANGER: &str = "c03-stranger-token-0123456789mnopqrstuvw";

use crate::support;

use support::{at, text};

struct Fixture {
    shared: support::StoreFixture,
}
impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("c02-tests"),
        };
        fs::create_dir(fixture.shared.root.join("published")).unwrap();
        fs::create_dir(fixture.shared.root.join("workspaces")).unwrap();
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
    fn token(&self, name: &str, text: &str) -> String {
        let path = self.shared.root.join(name);
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        options
            .open(&path)
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
        path.to_str().unwrap().to_owned()
    }
    fn head(&self) -> String {
        let store = self.path("published");
        text(
            &self.ok(&["open-revision", "--store", &store]),
            "revision_id",
        )
    }
    /// The published plate (created once) and a renamed workspace of `USER` named `name`, with
    /// its stored changeset: (digest, base revision).
    fn draft(&self, name: &str, label: &str) -> (String, String) {
        let store = self.path("published");
        if self.run(&["open-revision", "--store", &store]).0 != 0 {
            self.ok(&[
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
            ]);
        }
        let dir = self.path(&format!("workspaces/{name}"));
        self.ok(&[
            "create-workspace",
            "--store",
            &store,
            "--workspace-dir",
            &dir,
            "--user-id",
            USER,
        ]);
        self.ok(&[
            "update-element",
            "--store",
            &dir,
            "--id",
            PLATE,
            "--name",
            label,
        ]);
        let changeset = at(
            &self.ok(&[
                "create-changeset",
                "--store",
                &store,
                "--workspace-dir",
                &dir,
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
            .unwrap_or_else(|_| not_json(&line));
        assert_eq!(text(&listening, "outcome"), "listening", "{line}");
        Server {
            child,
            stdout,
            address: text(&listening, "address"),
        }
    }
}

fn not_json(line: &str) -> Json {
    Json::object([("outcome", Json::text(line.to_owned()))])
}

struct Server {
    child: Child,
    stdout: BufReader<ChildStdout>,
    address: String,
}
impl Server {
    /// Waits for the server to stop after its last request and returns its final JSON.
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

const TOKEN: &str = "c02-test-token-0123456789abcdefghijklmnop";
const OTHER_TOKEN: &str = "c02-other-token-0123456789abcdefghijklmno";

fn publish(
    f: &Fixture,
    server: &str,
    token: &str,
    workspace: &str,
    cs: &(String, String),
) -> (i32, Json) {
    f.run(&[
        "publish-changeset",
        "--address",
        server,
        "--token-file",
        token,
        "--workspace",
        workspace,
        "--changeset",
        &cs.0,
        "--expected-head",
        &cs.1,
    ])
}

#[test]
fn a_coordinator_needs_a_token_and_refuses_strangers() {
    let f = Fixture::new();
    let (store, root) = (f.path("published"), f.path("workspaces"));
    let serve = |extra: &[&str]| {
        let mut args = vec![
            "serve-coordinator",
            "--store",
            &store,
            "--workspaces-root",
            &root,
        ];
        args.extend_from_slice(extra);
        f.run(&args)
    };
    // Off unless explicitly started, and never without a client token.
    let (code, body) = serve(&[]);
    assert_eq!(
        (code, text(&body, "code").as_str()),
        (2, "INVALID_ARGUMENT")
    );
    let short = f.token("short", "too-short");
    let (code, body) = serve(&["--client", &format!("{USER}:{short}")]);
    assert_eq!(
        (code, text(&body, "code").as_str()),
        (2, "INVALID_ARGUMENT")
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let open = f.token("open", TOKEN);
        fs::set_permissions(&open, fs::Permissions::from_mode(0o644)).unwrap();
        let (code, body) = serve(&["--client", &format!("{USER}:{open}")]);
        assert_eq!(
            (code, text(&body, "code").as_str()),
            (2, "PERMISSION_DENIED")
        );
    }

    let token = f.token("token", TOKEN);
    let wrong = f.token("wrong", OTHER_TOKEN);
    let server = f.serve(&[(USER, &token)], 1);
    assert!(
        server.address.starts_with("127.0.0.1:"),
        "{}",
        server.address
    );
    // A wrong token is refused and not handled.
    let (code, body) = f.run(&[
        "inspect-coordinator-head",
        "--address",
        &server.address,
        "--token-file",
        &wrong,
    ]);
    assert_eq!(
        (code, text(&body, "code").as_str()),
        (2, "PERMISSION_DENIED")
    );
    // A non-loopback address is never contacted.
    let (code, body) = f.run(&[
        "inspect-coordinator-head",
        "--address",
        "192.0.2.1:9",
        "--token-file",
        &token,
    ]);
    assert_eq!(
        (code, text(&body, "code").as_str()),
        (2, "PERMISSION_DENIED")
    );
    let head = f.ok(&[
        "inspect-coordinator-head",
        "--address",
        &server.address,
        "--token-file",
        &token,
    ]);
    assert_eq!(text(&head, "outcome"), "head");
    assert_eq!(at(&head, &["revision_id"]), &Json::Null);
    let (code, stopped) = server.finish();
    assert_eq!(code, 0);
    assert_eq!(text(&stopped, "outcome"), "stopped");
}

#[test]
fn concurrent_publications_accept_one_and_never_overwrite_the_head() {
    let f = Fixture::new();
    let a = f.draft("a", "From A");
    let b = f.draft("b", "From B");
    assert_eq!(a.1, b.1, "both drafts share one base");
    let base = f.head();
    let token = f.token("token", TOKEN);
    let server = f.serve(&[(USER, &token)], 5);
    let address = server.address.clone();

    // Two clients publish at once from the same base.
    let results: Vec<(String, (i32, Json))> = std::thread::scope(|s| {
        let handles: Vec<_> = [("a", &a), ("b", &b)]
            .into_iter()
            .map(|(name, cs)| {
                let (f, address, token) = (&f, &address, &token);
                s.spawn(move || (name.to_owned(), publish(f, address, token, name, cs)))
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let accepted: Vec<_> = results
        .iter()
        .filter(|(_, (_, body))| text(body, "outcome") == "changeset_applied")
        .collect();
    let refused: Vec<_> = results
        .iter()
        .filter(|(_, (_, body))| text(body, "outcome") == "rejected")
        .collect();
    assert_eq!((accepted.len(), refused.len()), (1, 1), "{results:?}");
    let (winner, (code, applied)) = accepted[0];
    assert_eq!(*code, 0);
    let (_, (code, mismatch)) = refused[0];
    assert_eq!(*code, 2);
    assert_eq!(text(mismatch, "code"), "BASE_REVISION_MISMATCH");
    let new_head = text(applied, "revision_id");
    assert_ne!(new_head, base);
    assert_eq!(text(mismatch, "current_head"), new_head);
    assert_eq!(f.head(), new_head);

    // The same payload replays; the head stays the accepted one.
    let winner_cs = if winner == "a" { &a } else { &b };
    let store = beamcraft_host::revision::RevisionStore::new(
        beamcraft_host::storage::BlobStore::new(f.path("published")).unwrap(),
    );
    let captured = beamcraft_cli::command_evidence::read(&store.open().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(captured.fingerprint.to_string(), winner_cs.0);
    assert_eq!(
        captured.footprint,
        beamcraft_engine::footprint::Footprint::for_update(
            beamcraft_model::id::ElementId::parse(PLATE).unwrap(),
            true,
            false,
            false,
            false,
            None,
        )
        .unwrap()
    );
    let (code, replay) = publish(&f, &address, &token, winner, winner_cs);
    assert_eq!(
        (code, text(&replay, "outcome").as_str()),
        (0, "changeset_replayed")
    );
    assert_eq!(text(&replay, "revision_id"), new_head);
    // An expected head that is not the changeset's base is refused before anything is read.
    let loser = if winner == "a" { ("b", &b) } else { ("a", &a) };
    let stale = (loser.1.0.clone(), new_head.clone());
    let (code, body) = publish(&f, &address, &token, loser.0, &stale);
    assert_eq!(
        (code, text(&body, "code").as_str()),
        (2, "INVALID_ARGUMENT")
    );
    let head = f.ok(&[
        "inspect-coordinator-head",
        "--address",
        &address,
        "--token-file",
        &token,
    ]);
    assert_eq!(text(&head, "revision_id"), new_head);
    let (code, stopped) = server.finish();
    assert_eq!(code, 0);
    assert_eq!(text(&stopped, "outcome"), "stopped");
    assert_eq!(at(&stopped, &["handled"]), &Json::Number("5".into()));
    assert_eq!(f.head(), new_head);
}

#[test]
fn the_token_decides_the_actor_and_workspaces_stay_inside_the_root() {
    let f = Fixture::new();
    let cs = f.draft("mine", "Mine");
    let token = f.token("token", TOKEN);
    let stranger = f.token("stranger", OTHER_TOKEN);
    let outside = f.path("outside");
    fs::create_dir(&outside).unwrap();
    // The symbolic-link case runs on Unix only, so the request budget differs by platform.
    let requests = if cfg!(unix) { 4 } else { 3 };
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, f.shared.root.join("workspaces/escape")).unwrap();
    let server = f.serve(&[(USER, &token), (STRANGER, &stranger)], requests);
    let address = server.address.clone();
    let before = f.head();
    // USER's changeset under the stranger's token: the token is the trusted actor.
    let (code, body) = publish(&f, &address, &stranger, "mine", &cs);
    assert_eq!(
        (code, text(&body, "code").as_str()),
        (2, "PERMISSION_DENIED")
    );
    let (code, body) = publish(&f, &address, &token, "../published", &cs);
    assert_eq!(
        (code, text(&body, "code").as_str()),
        (2, "INVALID_ARGUMENT")
    );
    let (code, body) = publish(&f, &address, &token, "missing", &cs);
    assert_eq!((code, text(&body, "code").as_str()), (2, "NOT_FOUND"));
    #[cfg(unix)]
    {
        let (code, body) = publish(&f, &address, &token, "escape", &cs);
        assert_eq!(
            (code, text(&body, "code").as_str()),
            (2, "PERMISSION_DENIED")
        );
    }
    assert_eq!(f.head(), before);
    let (code, _) = server.finish();
    assert_eq!(code, 0);
}

/// C04-05 through the coordinator: a draft whose rebase recorded semantic conflicts cannot ship
/// on the wire either. Its already-packaged changeset is refused on the moved head, and only the
/// explicit resolution, the rebase onto the new head and a fresh re-edited changeset publish.
/// A's accepted edit is applied first and never rolled back.
#[test]
fn a_conflicted_draft_refuses_its_stale_changeset_until_resolved() {
    let f = Fixture::new();
    let published = f.path("published");
    let (cs, base) = f.draft("b", "local name");
    // A conflicting accepted edit moves the published head before B ships anything.
    let other = f.path("workspaces/a");
    f.ok(&[
        "create-workspace",
        "--store",
        &published,
        "--workspace-dir",
        &other,
        "--user-id",
        USER,
    ]);
    f.ok(&[
        "update-element",
        "--store",
        &other,
        "--id",
        PLATE,
        "--name",
        "remote name",
    ]);
    f.ok(&[
        "publish-workspace",
        "--store",
        &published,
        "--workspace-dir",
        &other,
    ]);
    let moved = f.head();
    // A's value is the accepted one; B's rename stays a draft.
    assert_eq!(plate_name(&published, PLATE), "remote name");

    let dir = f.path("workspaces/b");
    let draft_head = text(&f.ok(&["open-revision", "--store", &dir]), "revision_id");
    let (code, result) = f.run(&[
        "rebase-workspace",
        "--store",
        &published,
        "--workspace-dir",
        &dir,
        "--op-id",
        "019a7bf0-0000-7000-8000-00000000d501",
        "--expected-head",
        &moved,
        "--expected-draft",
        &draft_head,
    ]);
    assert_eq!(code, 2, "{result:?}");
    assert_eq!(text(&result, "code"), "DEPENDENCY_CONFLICT");
    let status = f.ok(&[
        "inspect-workspace",
        "--store",
        &published,
        "--workspace-dir",
        &dir,
    ]);
    let pending = at(&status, &["workspace", "pending_conflicts"])
        .array()
        .unwrap();
    let discarded = text(&pending[0], "draft_op").to_owned();

    // The stale changeset cannot ship on the moved head while the conflict stands.
    let token = f.token("token", TOKEN);
    let server = f.serve(&[(USER, &token)], 2);
    let address = server.address.clone();
    let (code, body) = publish(&f, &address, &token, "b", &(cs, base));
    assert_eq!(code, 2, "{body:?}");
    assert_eq!(text(&body, "outcome"), "rejected");
    assert_eq!(text(&body, "code"), "BASE_REVISION_MISMATCH");
    assert_eq!(f.head(), moved);

    // Resolution renounces B's edit, the rebase lands the accepted state, and a re-edited
    // changeset ships through the ordinary wire path.
    f.ok(&[
        "resolve-workspace",
        "--workspace-dir",
        &dir,
        "--op-id",
        "019a7bf0-0000-7000-8000-00000000d502",
        "--discard",
        &discarded,
        "--expected-draft",
        &draft_head,
    ]);
    f.ok(&[
        "rebase-workspace",
        "--store",
        &published,
        "--workspace-dir",
        &dir,
        "--op-id",
        "019a7bf0-0000-7000-8000-00000000d503",
        "--expected-head",
        &moved,
        "--expected-draft",
        &text(&f.ok(&["open-revision", "--store", &dir]), "revision_id"),
    ]);
    f.ok(&[
        "update-element",
        "--store",
        &dir,
        "--id",
        PLATE,
        "--translation",
        "10,0,0",
    ]);
    let packaged_json = f.ok(&[
        "create-changeset",
        "--store",
        &published,
        "--workspace-dir",
        &dir,
    ]);
    let packaged = at(&packaged_json, &["changeset"]);
    let fresh = (text(packaged, "digest"), text(packaged, "base_revision"));
    let (code, body) = publish(&f, &address, &token, "b", &fresh);
    assert_eq!(
        (code, text(&body, "outcome").as_str()),
        (0, "changeset_applied")
    );
    let new_head = text(&body, "revision_id");
    let (code, stopped) = server.finish();
    assert_eq!(code, 0);
    assert_eq!(text(&stopped, "outcome"), "stopped");

    // The accepted order is A's rename then B's move; no value was lost or rolled back.
    assert_eq!(f.head(), new_head);
    assert_eq!(plate_name(&published, PLATE), "remote name");
    let report = beamcraft_cli::plate::inspect(&beamcraft_host::revision::RevisionStore::new(
        beamcraft_host::storage::BlobStore::new(std::path::Path::new(&published)).unwrap(),
    ))
    .unwrap();
    let plate = report
        .plates
        .iter()
        .find(|p| p.authored.id == beamcraft_model::id::ElementId::parse(PLATE).unwrap())
        .unwrap();
    assert_eq!(plate.authored.name, "remote name");
    assert_eq!(plate.authored.placement.translation_mm, [10.0, 0.0, 0.0]);
}

fn plate_name(path: &str, id: &str) -> String {
    beamcraft_cli::plate::inspect(&beamcraft_host::revision::RevisionStore::new(
        beamcraft_host::storage::BlobStore::new(std::path::Path::new(path)).unwrap(),
    ))
    .unwrap()
    .plates
    .iter()
    .find(|p| p.authored.id == beamcraft_model::id::ElementId::parse(id).unwrap())
    .unwrap()
    .authored
    .name
    .clone()
}

fn plate_translation(path: &str, id: &str) -> [f64; 3] {
    beamcraft_cli::plate::inspect(&beamcraft_host::revision::RevisionStore::new(
        beamcraft_host::storage::BlobStore::new(std::path::Path::new(path)).unwrap(),
    ))
    .unwrap()
    .plates
    .iter()
    .find(|p| p.authored.id == beamcraft_model::id::ElementId::parse(id).unwrap())
    .unwrap()
    .authored
    .placement
    .translation_mm
}

/// Creates the published plate once.
impl Fixture {
    fn plate(&self) {
        let store = self.path("published");
        if self.run(&["open-revision", "--store", &store]).0 != 0 {
            self.ok(&[
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
            ]);
        }
    }
    /// A fresh workspace of `user` under `name`; returns its directory.
    fn workspace(&self, name: &str, user: &str) -> String {
        let dir = self.path(&format!("workspaces/{name}"));
        self.ok(&[
            "create-workspace",
            "--store",
            &self.path("published"),
            "--workspace-dir",
            &dir,
            "--user-id",
            user,
        ]);
        dir
    }
    /// The draft head revision of a workspace store.
    fn draft_head(&self, dir: &str) -> String {
        text(&self.ok(&["open-revision", "--store", dir]), "revision_id").to_owned()
    }
}

/// `rebase-coordinator` with the full request identity.
fn rebase(
    f: &Fixture,
    server: &str,
    token: &str,
    workspace: &str,
    op: &str,
    head: &str,
    draft: &str,
) -> (i32, Json) {
    f.run(&[
        "rebase-coordinator",
        "--address",
        server,
        "--token-file",
        token,
        "--workspace",
        workspace,
        "--op-id",
        op,
        "--expected-head",
        head,
        "--expected-draft",
        draft,
    ])
}

/// Packages a workspace's unsent work as (digest, base revision).
fn package(f: &Fixture, dir: &str) -> (String, String) {
    let changeset = at(
        &f.ok(&[
            "create-changeset",
            "--store",
            &f.path("published"),
            "--workspace-dir",
            dir,
        ]),
        &["changeset"],
    )
    .clone();
    (
        text(&changeset, "digest").to_owned(),
        text(&changeset, "base_revision").to_owned(),
    )
}

/// Sends one raw request frame and hangs up before the answer arrives: the client died after
/// the server accepted the request. Returns once the server's handling is observable.
fn send_and_drop(address: &str, token_text: &str, op: &str, fields: &[(&str, &str)]) {
    use std::collections::BTreeMap;
    use std::io::Write;
    use std::net::{Shutdown, TcpStream};
    let mut map = BTreeMap::new();
    map.insert("protocol".to_owned(), Json::text("beamcraft-coordinator/1"));
    map.insert("token".to_owned(), Json::text(token_text));
    map.insert("op".to_owned(), Json::text(op));
    for (key, value) in fields {
        map.insert((*key).to_owned(), Json::text(*value));
    }
    let body = Json::Object(map).encode(&Limits::DEFAULT).unwrap();
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .write_all(&(body.len() as u32).to_be_bytes())
        .unwrap();
    stream.write_all(&body).unwrap();
    stream.shutdown(Shutdown::Both).unwrap();
    drop(stream);
}

/// Waits until a workspace store's head is `revision` and its rebase boundary is finalized, so
/// the server finished handling the request and no lease is held against the retry.
fn wait_for_head(dir: &str, revision: &str) -> bool {
    for _ in 0..200 {
        let store = beamcraft_host::revision::RevisionStore::new(
            beamcraft_host::storage::BlobStore::new(std::path::Path::new(dir)).unwrap(),
        );
        let head_ok = matches!(
            store.head(),
            Ok(Some(ref info)) if info.revision().as_str() == revision
        );
        let meta_ok = fs::read(format!("{dir}/workspace.json"))
            .map(|bytes| {
                bytes
                    .windows("rebase_sequence".len())
                    .any(|w| w == b"rebase_sequence")
            })
            .unwrap_or(false);
        if head_ok && meta_ok {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    false
}

/// C03-R01, C03-R02, C03-R08: the wire rebase performs the local rebase semantics (independent
/// edits both survive, the published head is an input, a rebase never publishes), the token
/// decides the rebase actor, and one request identity replays across entry points.
#[test]
fn a_coordinator_rebase_keeps_both_edits_and_replays_across_entry_points() {
    let f = Fixture::new();
    f.plate();
    let published = f.path("published");
    // Both workspaces fork from one base before A publishes.
    let a_dir = f.workspace("a", USER);
    f.ok(&[
        "update-element",
        "--store",
        &a_dir,
        "--id",
        PLATE,
        "--name",
        "From A",
    ]);
    let b_dir = f.workspace("b", USER_B);
    f.ok(&[
        "update-element",
        "--store",
        &b_dir,
        "--id",
        PLATE,
        "--translation",
        "10,0,0",
    ]);
    f.ok(&[
        "publish-workspace",
        "--store",
        &published,
        "--workspace-dir",
        &a_dir,
    ]);
    let moved = f.head();
    let b_draft = f.draft_head(&b_dir);
    let metadata = fs::read(format!("{b_dir}/workspace.json")).unwrap();

    let token = f.token("token-a", TOKEN);
    let token_b = f.token("token-b", OTHER_TOKEN);
    let stranger = f.token("stranger", TOKEN_STRANGER);
    let server = f.serve(
        &[(USER, &token), (USER_B, &token_b), (STRANGER, &stranger)],
        6,
    );
    let address = server.address.clone();
    let op = "019a7bf0-0000-7000-8000-00000000d601";

    // C03-R02: a stranger's token is refused with nothing written anywhere.
    let (code, body) = rebase(&f, &address, &stranger, "b", op, &moved, &b_draft);
    assert_eq!(
        (code, text(&body, "code").as_str()),
        (2, "PERMISSION_DENIED")
    );
    assert_eq!(text(&body, "outcome"), "rejected");
    assert_eq!(f.head(), moved);
    assert_eq!(
        fs::read(format!("{b_dir}/workspace.json")).unwrap(),
        metadata
    );
    assert_eq!(f.draft_head(&b_dir), b_draft);
    // A request cannot name an actor: the unknown field is refused untouched.
    let raw = Json::object([
        ("protocol", Json::text("beamcraft-coordinator/1")),
        ("token", Json::text(OTHER_TOKEN)),
        ("op", Json::text("rebase")),
        ("workspace", Json::text("b")),
        ("op_id", Json::text(op)),
        ("expected_head", Json::text(&moved)),
        ("expected_draft", Json::text(&b_draft)),
        ("actor", Json::text(USER_B)),
    ]);
    {
        use std::io::{Read, Write};
        use std::net::TcpStream;
        let body = raw.encode(&Limits::DEFAULT).unwrap();
        let mut stream = TcpStream::connect(&address).unwrap();
        stream
            .write_all(&(body.len() as u32).to_be_bytes())
            .unwrap();
        stream.write_all(&body).unwrap();
        let mut length = [0u8; 4];
        stream.read_exact(&mut length).unwrap();
        let mut answer = vec![0u8; u32::from_be_bytes(length) as usize];
        stream.read_exact(&mut answer).unwrap();
        let answer = beamcraft_json::decode(&answer, &Limits::DEFAULT).unwrap();
        assert_eq!(text(&answer, "code"), "UNSUPPORTED_PROPERTY");
    }
    // Malformed identities are typed refusals before any state is read.
    let (code, body) = rebase(&f, &address, &token_b, "b", "not-a-uuid", &moved, &b_draft);
    assert_eq!(
        (code, text(&body, "code").as_str()),
        (2, "INVALID_ARGUMENT")
    );
    assert_eq!(f.head(), moved);

    // C03-R01: B rebases onto the moved head; the answer carries the local typed result.
    let (code, rebased) = rebase(&f, &address, &token_b, "b", op, &moved, &b_draft);
    assert_eq!(code, 0, "{rebased:?}");
    assert_eq!(text(&rebased, "outcome"), "workspace_rebased");
    assert_eq!(
        text(rebased.get("revision").unwrap(), "revision_id"),
        op,
        "the candidate revision is the request identity"
    );
    assert_eq!(text(&rebased, "base_revision"), moved);
    assert_eq!(text(&rebased, "retained_operations"), "1");
    assert_eq!(text(&rebased, "current_head"), moved);
    assert_eq!(text(&rebased, "current_head_status"), "known");
    assert_eq!(f.head(), moved, "a rebase never publishes");
    assert_eq!(plate_name(&b_dir, PLATE), "From A");
    assert_eq!(plate_translation(&b_dir, PLATE), [10.0, 0.0, 0.0]);

    // The rebased draft ships through the ordinary wire publish against its new base.
    let cs = package(&f, &b_dir);
    assert_eq!(cs.1, moved);
    // C03-R02: the changeset keeps the workspace actor as its author, so a stranger's token
    // cannot publish it while B's own token can.
    let (code, stolen) = publish(&f, &address, &stranger, "b", &cs);
    assert_eq!(
        (code, text(&stolen, "code").as_str()),
        (2, "PERMISSION_DENIED")
    );
    assert_eq!(f.head(), moved);
    let (code, applied) = publish(&f, &address, &token_b, "b", &cs);
    assert_eq!(
        (code, text(&applied, "outcome").as_str()),
        (0, "changeset_applied")
    );
    let new_head = text(&applied, "revision_id");
    assert_eq!(f.head(), new_head);
    assert_eq!(plate_name(&published, PLATE), "From A");
    assert_eq!(plate_translation(&published, PLATE), [10.0, 0.0, 0.0]);

    // C03-R08: the same identity through the local CLI replays the recorded outcome.
    let (code, replay) = f.run(&[
        "rebase-workspace",
        "--store",
        &published,
        "--workspace-dir",
        &b_dir,
        "--op-id",
        op,
        "--expected-head",
        &moved,
        "--expected-draft",
        &b_draft,
    ]);
    assert_eq!(code, 0, "{replay:?}");
    assert_eq!(text(&replay, "outcome"), "workspace_rebase_replayed");
    assert_eq!(text(replay.get("revision").unwrap(), "revision_id"), op);
    let (code, stopped) = server.finish();
    assert_eq!(code, 0);
    assert_eq!(text(&stopped, "outcome"), "stopped");
}

/// C03-R04, C03-R05: an interleaved publication makes the loser rebase again and publish against
/// the observed head; concurrent rebases of one workspace merge exactly once.
#[test]
fn an_interleaved_publication_and_a_rebase_race_keep_the_accepted_order() {
    let f = Fixture::new();
    f.plate();
    let published = f.path("published");
    // A second element for c: its thickness edit must stay independent of the move's shape read.
    f.ok(&[
        "create-plate",
        "--store",
        &published,
        "--width",
        "800",
        "--length",
        "400",
        "--thickness",
        "8",
        "--new-id",
        PLATE2,
    ]);
    // All three workspaces fork from one base before anyone publishes.
    let a_dir = f.workspace("a", USER);
    f.ok(&[
        "update-element",
        "--store",
        &a_dir,
        "--id",
        PLATE,
        "--name",
        "From A",
    ]);
    let b_dir = f.workspace("b", USER_B);
    f.ok(&[
        "update-element",
        "--store",
        &b_dir,
        "--id",
        PLATE,
        "--translation",
        "10,0,0",
    ]);
    let b_draft = f.draft_head(&b_dir);
    let c_dir = f.workspace("c", USER_C);
    f.ok(&[
        "update-element",
        "--store",
        &c_dir,
        "--id",
        PLATE2,
        "--thickness",
        "12",
    ]);
    let c_draft = f.draft_head(&c_dir);
    let token = f.token("token-a", TOKEN);
    let token_b = f.token("token-b", OTHER_TOKEN);
    let token_c = f.token("token-c", TOKEN_C);
    let server = f.serve(&[(USER, &token), (USER_B, &token_b), (USER_C, &token_c)], 7);
    let address = server.address.clone();
    let (op1, op2) = (
        "019a7bf0-0000-7000-8000-00000000d611",
        "019a7bf0-0000-7000-8000-00000000d612",
    );

    // A publishes, B rebases through the coordinator, A publishes again.
    f.ok(&[
        "publish-workspace",
        "--store",
        &published,
        "--workspace-dir",
        &a_dir,
    ]);
    let head1 = f.head();
    let (code, rebased) = rebase(&f, &address, &token_b, "b", op1, &head1, &b_draft);
    assert_eq!(
        (code, text(&rebased, "outcome").as_str()),
        (0, "workspace_rebased")
    );
    let cs1 = package(&f, &b_dir);
    assert_eq!(cs1.1, head1);
    f.ok(&[
        "update-element",
        "--store",
        &a_dir,
        "--id",
        PLATE,
        "--name",
        "From A2",
    ]);
    f.ok(&[
        "publish-workspace",
        "--store",
        &published,
        "--workspace-dir",
        &a_dir,
    ]);
    let head2 = f.head();

    // B's stale publish is refused with the current head and nothing written.
    let (code, stale) = publish(&f, &address, &token_b, "b", &cs1);
    assert_eq!(
        (code, text(&stale, "code").as_str()),
        (2, "BASE_REVISION_MISMATCH")
    );
    assert_eq!(text(&stale, "outcome"), "rejected");
    assert_eq!(text(&stale, "current_head"), head2);
    assert_eq!(f.head(), head2);

    // B repeats the rebase under a fresh identity, then publishes against the observed head.
    let b_draft2 = f.draft_head(&b_dir);
    let (code, rebased2) = rebase(&f, &address, &token_b, "b", op2, &head2, &b_draft2);
    assert_eq!(
        (code, text(&rebased2, "outcome").as_str()),
        (0, "workspace_rebased")
    );
    assert_eq!(text(&rebased2, "base_revision"), head2);
    let cs2 = package(&f, &b_dir);
    assert_eq!(cs2.1, head2);
    let (code, applied) = publish(&f, &address, &token_b, "b", &cs2);
    assert_eq!(
        (code, text(&applied, "outcome").as_str()),
        (0, "changeset_applied")
    );
    let head3 = text(&applied, "revision_id");
    assert_eq!(f.head(), head3);
    assert_eq!(plate_name(&published, PLATE), "From A2");
    assert_eq!(plate_translation(&published, PLATE), [10.0, 0.0, 0.0]);

    // C03-R05: two concurrent rebase requests of one workspace with one identity merge once;
    // the queue answers the second from the record.
    let op3 = "019a7bf0-0000-7000-8000-00000000d613";
    let answers: Vec<Json> = std::thread::scope(|s| {
        let attempts: Vec<_> = (0..2)
            .map(|_| {
                let (f, address, token_c, head, draft) = (&f, &address, &token_c, &head3, &c_draft);
                s.spawn(move || rebase(f, address, token_c, "c", op3, head, draft).1)
            })
            .collect();
        attempts.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let outcomes: Vec<String> = answers.iter().map(|a| text(a, "outcome")).collect();
    assert!(
        outcomes.contains(&"workspace_rebased".to_owned())
            && outcomes.contains(&"workspace_rebase_replayed".to_owned()),
        "{outcomes:?}"
    );
    for answer in &answers {
        assert_eq!(text(answer.get("revision").unwrap(), "revision_id"), op3);
        assert_eq!(text(answer, "base_revision"), head3);
    }
    assert_eq!(f.head(), head3, "the race never touched the published head");
    assert_eq!(plate_translation(&c_dir, PLATE), [10.0, 0.0, 0.0]);
    assert_eq!(plate_name(&c_dir, PLATE), "From A2");
    assert_eq!(thickness(&c_dir, PLATE2), 12.0);
    // A different identity against the merged draft is a typed refusal, never a second merge.
    let (code, refused) = rebase(&f, &address, &token_c, "c", op2, &head3, op3);
    assert_eq!(
        (code, text(&refused, "code").as_str()),
        (2, "INVALID_ARGUMENT")
    );
    assert_eq!(f.draft_head(&c_dir), op3);
    let (code, stopped) = server.finish();
    assert_eq!(code, 0);
    assert_eq!(text(&stopped, "outcome"), "stopped");
    assert_eq!(at(&stopped, &["handled"]), &Json::Number("7".into()));
}

/// C03-R06: a rebase survives every crash window. A client that dies after the server accepted
/// the request, a server killed before the commit and a server killed between the draft commit
/// and the answer all end in the same recorded outcome, replayed exactly once by a retry.
#[test]
fn a_rebase_survives_crash_windows_and_replays_exactly_once() {
    use beamcraft_cli::workspace::rebase::{RebaseRequest, rebase_workspace};
    use beamcraft_host::revision::CommitPoint;

    struct Crash;
    let f = Fixture::new();
    f.plate();
    let published = f.path("published");
    // All workspaces fork from one base; A publishes once and the rebases follow.
    let a_dir = f.workspace("a", USER);
    f.ok(&[
        "update-element",
        "--store",
        &a_dir,
        "--id",
        PLATE,
        "--name",
        "From A",
    ]);
    let b_dir = f.workspace("b", USER_B);
    f.ok(&[
        "update-element",
        "--store",
        &b_dir,
        "--id",
        PLATE,
        "--translation",
        "10,0,0",
    ]);
    let b_draft = f.draft_head(&b_dir);
    let c_dir = f.workspace("c", USER_B);
    f.ok(&[
        "update-element",
        "--store",
        &c_dir,
        "--id",
        PLATE,
        "--thickness",
        "12",
    ]);
    let c_draft = f.draft_head(&c_dir);
    let d_dir = f.workspace("d", USER_B);
    f.ok(&[
        "update-element",
        "--store",
        &d_dir,
        "--id",
        PLATE,
        "--thickness",
        "8",
    ]);
    let d_draft = f.draft_head(&d_dir);
    f.ok(&[
        "publish-workspace",
        "--store",
        &published,
        "--workspace-dir",
        &a_dir,
    ]);
    let moved = f.head();
    let token = f.token("token-a", TOKEN);
    let token_b = f.token("token-b", OTHER_TOKEN);
    let server = f.serve(&[(USER, &token), (USER_B, &token_b)], 5);
    let address = server.address.clone();

    // The client dies after the server accepted the request, before the answer.
    let op1 = "019a7bf0-0000-7000-8000-00000000d621";
    send_and_drop(
        &address,
        OTHER_TOKEN,
        "rebase",
        &[
            ("workspace", "b"),
            ("op_id", op1),
            ("expected_head", &moved),
            ("expected_draft", &b_draft),
        ],
    );
    assert!(
        wait_for_head(&b_dir, op1),
        "the coordinator never committed the rebase request"
    );
    let (code, replay) = rebase(&f, &address, &token_b, "b", op1, &moved, &b_draft);
    assert_eq!(
        (code, text(&replay, "outcome").as_str()),
        (0, "workspace_rebase_replayed")
    );
    assert_eq!(text(replay.get("revision").unwrap(), "revision_id"), op1);
    assert_eq!(plate_name(&b_dir, PLATE), "From A");
    assert_eq!(plate_translation(&b_dir, PLATE), [10.0, 0.0, 0.0]);

    // The server dies before the commit: nothing is merged, the retry commits exactly once.
    let op2 = "019a7bf0-0000-7000-8000-00000000d622";
    let killed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        rebase_workspace(
            &beamcraft_host::revision::RevisionStore::new(
                beamcraft_host::storage::BlobStore::new(std::path::Path::new(&published)).unwrap(),
            ),
            &beamcraft_host::revision::RevisionStore::new(
                beamcraft_host::storage::BlobStore::new(std::path::Path::new(&c_dir)).unwrap(),
            ),
            std::path::Path::new(&c_dir),
            &RebaseRequest {
                op_id: beamcraft_model::id::OpId::parse(op2).unwrap(),
                actor: beamcraft_host::revision::Uuid7::parse(USER_B).unwrap(),
                expected_head: beamcraft_host::revision::Uuid7::parse(&moved).unwrap(),
                expected_draft: beamcraft_host::revision::Uuid7::parse(&c_draft).unwrap(),
            },
            &mut |reached: CommitPoint| {
                if reached == CommitPoint::IntentRecorded {
                    std::panic::resume_unwind(Box::new(Crash));
                }
                Ok(())
            },
        )
    }));
    assert!(killed.is_err(), "the injected crash did not happen");
    assert_eq!(
        f.draft_head(&c_dir),
        c_draft,
        "nothing was merged before the crash"
    );
    // The operator closes the interrupted commit (U07), then the wire retry runs once.
    f.ok(&["recover-revision", "--store", &c_dir, "--min-age-secs", "0"]);
    let (code, rebased) = rebase(&f, &address, &token_b, "c", op2, &moved, &c_draft);
    assert_eq!(
        (code, text(&rebased, "outcome").as_str()),
        (0, "workspace_rebased")
    );
    assert_eq!(f.draft_head(&c_dir), op2);
    assert_eq!(
        thickness(&c_dir, PLATE),
        12.0,
        "the unsent edit survived the crash window"
    );
    let (code, replayed) = rebase(&f, &address, &token_b, "c", op2, &moved, &c_draft);
    assert_eq!(
        (code, text(&replayed, "outcome").as_str()),
        (0, "workspace_rebase_replayed")
    );

    // The server dies between the draft commit and the answer: the metadata is unfinished, the
    // retry finalizes the recovery and answers from the record, still exactly once.
    let op3 = "019a7bf0-0000-7000-8000-00000000d623";
    let killed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        rebase_workspace(
            &beamcraft_host::revision::RevisionStore::new(
                beamcraft_host::storage::BlobStore::new(std::path::Path::new(&published)).unwrap(),
            ),
            &beamcraft_host::revision::RevisionStore::new(
                beamcraft_host::storage::BlobStore::new(std::path::Path::new(&d_dir)).unwrap(),
            ),
            std::path::Path::new(&d_dir),
            &RebaseRequest {
                op_id: beamcraft_model::id::OpId::parse(op3).unwrap(),
                actor: beamcraft_host::revision::Uuid7::parse(USER_B).unwrap(),
                expected_head: beamcraft_host::revision::Uuid7::parse(&moved).unwrap(),
                expected_draft: beamcraft_host::revision::Uuid7::parse(&d_draft).unwrap(),
            },
            &mut |reached: CommitPoint| {
                if reached == CommitPoint::Switched {
                    std::panic::resume_unwind(Box::new(Crash));
                }
                Ok(())
            },
        )
    }));
    assert!(killed.is_err(), "the injected crash did not happen");
    assert_eq!(
        f.draft_head(&d_dir),
        op3,
        "the candidate committed before the crash"
    );
    let (code, replay) = rebase(&f, &address, &token_b, "d", op3, &moved, &d_draft);
    assert_eq!(
        (code, text(&replay, "outcome").as_str()),
        (0, "workspace_rebase_replayed")
    );
    assert_eq!(text(replay.get("revision").unwrap(), "revision_id"), op3);
    assert_eq!(f.draft_head(&d_dir), op3, "no second merge happened");
    assert_eq!(plate_name(&d_dir, PLATE), "From A");
    let (code, stopped) = server.finish();
    assert_eq!(code, 0);
    assert_eq!(text(&stopped, "outcome"), "stopped");
    assert_eq!(at(&stopped, &["handled"]), &Json::Number("5".into()));
}

/// A plate thickness as the strict inspection reports it.
fn thickness(path: &str, id: &str) -> f64 {
    beamcraft_cli::plate::inspect(&beamcraft_host::revision::RevisionStore::new(
        beamcraft_host::storage::BlobStore::new(std::path::Path::new(path)).unwrap(),
    ))
    .unwrap()
    .plates
    .iter()
    .find(|p| p.authored.id == beamcraft_model::id::ElementId::parse(id).unwrap())
    .unwrap()
    .authored
    .thickness_mm
}

/// C03-R01, C03-R03: a conflicting rebase is answered with the complete semantic diagnostics
/// byte-stably, writes nothing, and authentication failures never touch any state.
#[test]
fn a_conflicting_rebase_answers_diagnostics_byte_stably_and_authenticates_first() {
    let f = Fixture::new();
    f.plate();
    let published = f.path("published");
    // Both workspaces fork from one base before A publishes its conflicting rename.
    let a_dir = f.workspace("a", USER);
    f.ok(&[
        "update-element",
        "--store",
        &a_dir,
        "--id",
        PLATE,
        "--name",
        "remote name",
    ]);
    let b_dir = f.workspace("b", USER_B);
    f.ok(&[
        "update-element",
        "--store",
        &b_dir,
        "--id",
        PLATE,
        "--name",
        "local name",
    ]);
    f.ok(&[
        "publish-workspace",
        "--store",
        &published,
        "--workspace-dir",
        &a_dir,
    ]);
    let moved = f.head();
    let b_draft = f.draft_head(&b_dir);
    let metadata = fs::read(format!("{b_dir}/workspace.json")).unwrap();

    let token_b = f.token("token-b", OTHER_TOKEN);
    let server = f.serve(&[(USER_B, &token_b)], 2);
    let address = server.address.clone();
    let op = "019a7bf0-0000-7000-8000-00000000d631";

    // C03-R03: unknown tokens are refused by the server before any workspace state is read;
    // heads and both drafts stay exactly as they were.
    let (code, body) = rebase(
        &f,
        &address,
        &f.token("bad", "not-the-token-0123456789abcdefghij"),
        "b",
        op,
        &moved,
        &b_draft,
    );
    assert_eq!(
        (code, text(&body, "code").as_str()),
        (2, "PERMISSION_DENIED")
    );
    assert_eq!(f.head(), moved);
    assert_eq!(f.draft_head(&b_dir), b_draft);

    let (code, conflict) = rebase(&f, &address, &token_b, "b", op, &moved, &b_draft);
    assert_eq!(code, 2, "{conflict:?}");
    assert_eq!(text(&conflict, "outcome"), "rejected");
    assert_eq!(text(&conflict, "code"), "DEPENDENCY_CONFLICT");
    let diagnostics = conflict.get("diagnostics").unwrap().array().unwrap();
    assert!(!diagnostics.is_empty(), "{conflict:?}");
    assert_eq!(text(&conflict, "current_head"), moved);
    assert_eq!(f.head(), moved);
    assert_eq!(f.draft_head(&b_dir), b_draft);
    assert_eq!(
        fs::read(format!("{b_dir}/workspace.json")).unwrap(),
        metadata
    );
    assert_eq!(plate_name(&b_dir, PLATE), "local name");
    assert_eq!(plate_name(&published, PLATE), "remote name");

    // The retry answers byte-stably from the recorded report.
    let (code, again) = rebase(&f, &address, &token_b, "b", op, &moved, &b_draft);
    assert_eq!(code, 2);
    assert_eq!(again, conflict);
    let (code, stopped) = server.finish();
    assert_eq!(code, 0);
    assert_eq!(text(&stopped, "outcome"), "stopped");
}
