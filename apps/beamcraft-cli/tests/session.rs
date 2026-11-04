// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! `session` end to end (`I424`): a real `beamcraft session` process over pipes, driven one
//! command line at a time, with the store checked from separate one-shot processes. The
//! scenarios follow the design note: unbracketed commands behave exactly as today, a batch
//! publishes one revision or nothing, a refusal abandons the batch whole with the session
//! alive, a moved base fails the batch, and a carried command replays by its own operation
//! id from a later process.

use crate::support::{at, text};
use beamcraft_json::{Json, Limits, decode};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

struct Store(PathBuf);

impl Store {
    fn path(&self) -> &Path {
        &self.0
    }

    fn arg(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }

    /// Runs one one-shot command and decodes its JSON output. The arguments carry their own
    /// `--store`, exactly as a session line does.
    fn run(&self, args: &[&str]) -> Json {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .output()
            .unwrap();
        decode(&output.stdout, &Limits::DEFAULT).unwrap()
    }

    /// The head-log sequence the store is at.
    fn sequence(&self) -> u64 {
        let summary = self.run(&["open-revision", "--store", &self.arg()]);
        summary
            .get("sequence")
            .unwrap()
            .string()
            .unwrap()
            .parse()
            .unwrap()
    }
}

struct Session {
    child: Child,
    out: BufReader<std::process::ChildStdout>,
}

impl Session {
    fn start(store: &Store) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .arg("session")
            .arg("--store")
            .arg(store.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        let mut session = Self { child, out };
        let ready = session.line();
        assert_eq!(
            ready.get("outcome").unwrap().string().unwrap(),
            "session_ready"
        );
        session
    }

    /// Sends one line and reads the one answer line.
    fn line(&mut self) -> Json {
        let mut text = String::new();
        self.out.read_line(&mut text).unwrap();
        assert!(!text.is_empty(), "the session closed its output");
        decode(text.trim_end().as_bytes(), &Limits::DEFAULT).unwrap()
    }

    fn ask(&mut self, command: &str) -> Json {
        writeln!(self.child.stdin.as_mut().unwrap(), "{command}").unwrap();
        self.line()
    }

    /// Sends one line and reads two answer lines (a refusal plus the batch abandonment).
    fn ask_refused_in_batch(&mut self, command: &str) -> (Json, Json) {
        writeln!(self.child.stdin.as_mut().unwrap(), "{command}").unwrap();
        let refusal = self.line();
        let abandoned = self.line();
        (refusal, abandoned)
    }

    fn outcome(json: &Json) -> String {
        json.get("outcome").unwrap().string().unwrap().to_owned()
    }

    /// Any scalar (string or number token) as its text.
    fn scalar(json: &Json, key: &str) -> String {
        let value = json.get(key).unwrap();
        match value.string() {
            Ok(text) => text.to_owned(),
            Err(_) => match value {
                Json::Number(token) => token.clone(),
                other => panic!("{key}: unexpected shape {other:?}"),
            },
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn create_plate(store: &str, id: &str, op: &str, thickness: &str) -> String {
    format!(
        "create-plate --store {store} --new-id {id} --op-id {op} --width 200 --length 200 --thickness {thickness} --name batch"
    )
}

fn fresh() -> (PathBuf, Store) {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let root = std::env::temp_dir().join(format!(
        "beamcraft-session-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    // The store root must exist before the first command opens it.
    std::fs::create_dir(root.join("store")).unwrap();
    let store = Store(root.join("store"));
    (root, store)
}

#[test]
fn a_session_runs_commands_batches_and_refusals_like_the_design_says() {
    let (_root, store) = fresh();
    let root = store.arg();
    // The first revision belongs outside a batch; one one-shot create sets it.
    let first = format!(
        "create-plate --store {root} --new-id 019a7bf0-0000-7000-8000-000000000001 --op-id 019a7bf0-0000-7000-8000-000000000002 --width 200 --length 200 --thickness 8 --name first"
    );
    let created = store.run(first.split_whitespace().collect::<Vec<_>>().as_slice());
    assert_eq!(Session::outcome(&created), "plate_created");
    let base = store.sequence();
    assert_eq!(base, 1);

    let mut session = Session::start(&store);
    // Unbracketed command: the store moves exactly as it would one process per command.
    let answer = session.ask(&format!(
        "create-plate --store {root} --new-id 019a7bf0-0000-7000-8000-000000000003 --op-id 019a7bf0-0000-7000-8000-000000000004 --width 200 --length 200 --thickness 8 --name second"
    ));
    assert_eq!(Session::outcome(&answer), "plate_created");
    assert_eq!(store.sequence(), 2);

    // A batch of two commands publishes one revision, and nothing before its commit.
    assert_eq!(Session::outcome(&session.ask("batch-open")), "batch_opened");
    let carried_a = create_plate(
        &root,
        "019a7bf0-0000-7000-8000-000000000005",
        "019a7bf0-0000-7000-8000-000000000006",
        "10",
    );
    assert_eq!(Session::outcome(&session.ask(&carried_a)), "plate_created");
    let carried_b = create_plate(
        &root,
        "019a7bf0-0000-7000-8000-000000000007",
        "019a7bf0-0000-7000-8000-000000000008",
        "12",
    );
    assert_eq!(Session::outcome(&session.ask(&carried_b)), "plate_created");
    assert_eq!(
        store.sequence(),
        2,
        "nothing between the brackets hits the disk"
    );
    let committed = session.ask(
        "batch-commit --op-id 019a7bf0-0000-7000-8000-000000000009 --revision-id 019a7bf0-0000-7000-8000-00000000000a",
    );
    assert_eq!(Session::outcome(&committed), "batch_committed");
    assert_eq!(
        Session::scalar(&committed, "commands"),
        "2",
        "the batch reports how many commands it carried"
    );
    assert_eq!(
        committed
            .get("revision")
            .unwrap()
            .get("sequence")
            .unwrap()
            .string()
            .unwrap(),
        "3"
    );
    assert_eq!(store.sequence(), 3);

    // A carried command replays by its own operation id from a later process: the same
    // request is answered from the batch revision, a different one is reuse.
    let replay = store.run(carried_a.split_whitespace().collect::<Vec<_>>().as_slice());
    // The outcome string itself is the replay flag: the same request under the carried
    // command's operation id is answered, not re-executed.
    assert_eq!(Session::outcome(&replay), "plate_replayed");
    let reuse = carried_a.replace(" --thickness 10", " --thickness 11");
    let refused = store.run(reuse.split_whitespace().collect::<Vec<_>>().as_slice());
    assert_eq!(
        Session::outcome(&refused),
        "rejected",
        "reuse after the batch: {refused:?}"
    );
    assert_eq!(
        refused.get("code").unwrap().string().unwrap(),
        "OP_ID_REUSE"
    );

    // A refusal inside a batch abandons it whole; the session stays alive.
    assert_eq!(Session::outcome(&session.ask("batch-open")), "batch_opened");
    let good = create_plate(
        &root,
        "019a7bf0-0000-7000-8000-00000000000b",
        "019a7bf0-0000-7000-8000-00000000000c",
        "10",
    );
    assert_eq!(Session::outcome(&session.ask(&good)), "plate_created");
    let (refusal, abandoned) = session.ask_refused_in_batch("create-plate --nonsense");
    assert_eq!(Session::outcome(&refusal), "rejected");
    assert_eq!(Session::outcome(&abandoned), "batch_abandoned");
    let closed = session.ask("batch-commit");
    assert_eq!(Session::outcome(&closed), "rejected");
    assert_eq!(store.sequence(), 3, "the abandoned batch left no trace");
    // The session is ready for a new batch right away.
    assert_eq!(Session::outcome(&session.ask("batch-open")), "batch_opened");
    assert_eq!(
        Session::outcome(&session.ask("batch-abort")),
        "batch_abandoned"
    );

    // A moved base fails the batch instead of rebasing it.
    assert_eq!(Session::outcome(&session.ask("batch-open")), "batch_opened");
    let outside = format!(
        "create-plate --store {root} --new-id 019a7bf0-0000-7000-8000-00000000000d --op-id 019a7bf0-0000-7000-8000-00000000000e --width 200 --length 200 --thickness 8 --name outside"
    );
    store.run(outside.split_whitespace().collect::<Vec<_>>().as_slice());
    let draft = create_plate(
        &root,
        "019a7bf0-0000-7000-8000-00000000000f",
        "019a7bf0-0000-7000-8000-000000000010",
        "10",
    );
    assert_eq!(Session::outcome(&session.ask(&draft)), "plate_created");
    let failed = session.ask("batch-commit");
    assert_eq!(Session::outcome(&failed), "rejected");
    assert_eq!(
        store.sequence(),
        4,
        "the outside commit landed, the batch did not"
    );

    // Only modelling commands may run between the brackets.
    assert_eq!(Session::outcome(&session.ask("batch-open")), "batch_opened");
    let (refusal, abandoned) = session.ask_refused_in_batch(
        "create-project --store /elsewhere/other --dir /tmp/beamcraft-session-other",
    );
    assert_eq!(Session::outcome(&refusal), "rejected");
    assert_eq!(Session::outcome(&abandoned), "batch_abandoned");

    // A repeated operation id inside one batch is refused before any work.
    assert_eq!(Session::outcome(&session.ask("batch-open")), "batch_opened");
    let dup = create_plate(
        &root,
        "019a7bf0-0000-7000-8000-000000000011",
        "019a7bf0-0000-7000-8000-000000000012",
        "10",
    );
    assert_eq!(Session::outcome(&session.ask(&dup)), "plate_created");
    let (refusal, abandoned) = session.ask_refused_in_batch(&dup);
    assert_eq!(
        refusal.get("code").unwrap().string().unwrap(),
        "OP_ID_REUSE"
    );
    assert_eq!(Session::outcome(&abandoned), "batch_abandoned");

    // Ending the session with an open batch abandons it and says so.
    assert_eq!(Session::outcome(&session.ask("batch-open")), "batch_opened");
    drop(session.child.stdin.take());
    let last = session.line();
    assert_eq!(Session::outcome(&last), "batch_abandoned");
    let status = session.child.wait().unwrap();
    assert!(status.success());
    assert_eq!(store.sequence(), 4);
}

#[test]
fn a_definition_the_head_already_holds_answers_inside_a_batch() {
    let (_root, store) = fresh();
    let root = store.arg();
    // The first revision belongs outside a batch: one material definition sets it.
    let material = format!("define-material --store {root} --density 7850");
    let created = store.run(material.split_whitespace().collect::<Vec<_>>().as_slice());
    assert_eq!(Session::outcome(&created), "definition_created");

    let mut session = Session::start(&store);
    assert_eq!(Session::outcome(&session.ask("batch-open")), "batch_opened");
    // The head already holds this exact definition: the same answer as outside the batch,
    // and the batch survives the command (an integrity loss used to abandon it).
    let again = session.ask(&material);
    assert_eq!(Session::outcome(&again), "definition_exists");
    assert_eq!(
        Session::outcome(&session.ask("batch-abort")),
        "batch_abandoned"
    );
}

#[test]
fn an_open_session_holds_the_compaction_lease() {
    let (_root, store) = fresh();
    let root = store.arg();
    // The first revision belongs outside a batch: one plate sets it.
    let first = format!(
        "create-plate --store {root} --new-id 019a7bf0-0000-7000-8000-000000000001 --op-id 019a7bf0-0000-7000-8000-000000000002 --width 200 --length 200 --thickness 8 --name first"
    );
    let created = store.run(first.split_whitespace().collect::<Vec<_>>().as_slice());
    assert_eq!(Session::outcome(&created), "plate_created");

    let session = Session::start(&store);
    // The open session holds the shared store lease, so compaction refuses while it serves
    // (U14): a compaction used to be able to start under a live session, including the
    // middle of a batch.
    let compact = store.run(&[
        "compact-history",
        "--store",
        &root,
        "--keep-head",
        "--dry-run",
    ]);
    assert_eq!(Session::outcome(&compact), "compaction_dry_run");
    assert!(!compact.get("would_proceed").unwrap().boolean().unwrap());
    assert_eq!(
        text(&at(&compact, &["blockers"]).array().unwrap()[0], "kind"),
        "store_in_use"
    );

    // Ending the session releases the lease and compaction runs again.
    drop(session);
    let after = store.run(&[
        "compact-history",
        "--store",
        &root,
        "--keep-head",
        "--dry-run",
    ]);
    assert_eq!(Session::outcome(&after), "compaction_dry_run");
    assert!(after.get("would_proceed").unwrap().boolean().unwrap());
}
