// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, workspaces and tokens, MPL-2.0.

//! C02 end to end: a real `coordinator-serve` process and `coordinator-publish` /
//! `coordinator-head` clients. The coordinator does not start without a token; a wrong token is
//! refused; the token decides the actor; two concurrent publications from one base accept exactly
//! one and the accepted head is never overwritten; the same payload replays; workspace names stay
//! inside the workspaces root.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_json::{Json, Limits};
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

const USER: &str = "019a7bf0-0000-7000-8000-00000000c201";
const STRANGER: &str = "019a7bf0-0000-7000-8000-00000000c202";
const PLATE: &str = "019a7bf0-0000-7000-8000-000000000210";

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("c02-tests");
        fs::create_dir_all(&base).unwrap();
        let root = base.join(format!(
            "e2e-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("published")).unwrap();
        fs::create_dir(root.join("workspaces")).unwrap();
        Self { root }
    }
    fn path(&self, name: &str) -> String {
        self.root.join(name).to_str().unwrap().to_owned()
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
        let path = self.root.join(name);
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
            &self.ok(&["revision-open", "--store", &store]),
            "revision_id",
        )
    }
    /// The published plate (created once) and a renamed workspace of `USER` named `name`, with
    /// its stored changeset: (digest, base revision).
    fn draft(&self, name: &str, label: &str) -> (String, String) {
        let store = self.path("published");
        if self.run(&["revision-open", "--store", &store]).0 != 0 {
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
            "workspace-create",
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
                "changeset-create",
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
            "coordinator-serve".to_owned(),
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
impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.root.is_absolute() && self.root.file_name().is_some());
        let _ = fs::remove_dir_all(&self.root);
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

fn text(json: &Json, key: &str) -> String {
    json.get(key).unwrap().string().unwrap().to_owned()
}
fn at<'a>(json: &'a Json, path: &[&str]) -> &'a Json {
    path.iter().fold(json, |j, k| j.get(k).unwrap())
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
        "coordinator-publish",
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
            "coordinator-serve",
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
        "coordinator-head",
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
        "coordinator-head",
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
        "coordinator-head",
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
        "coordinator-head",
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
    std::os::unix::fs::symlink(&outside, f.root.join("workspaces/escape")).unwrap();
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
