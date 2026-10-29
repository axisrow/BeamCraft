// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, workspaces and tokens, MPL-2.0.

//! C05 end to end: two offline clients against one published store, through the real
//! `beamcraft` binary and a real `serve-coordinator` process (the C05 rows of
//! `docs/design/c03-c06-acceptance.md`). Every case asserts content — values, revisions,
//! blob counts — never only an exit code, and every adversarial path ends in a typed
//! result: no lost writes and deterministic operation outcomes across retries, crash
//! windows, forced identity collisions and bounded failures.
//!
//! The element count per workspace is bounded by the O(revisions) verification cost of
//! every commit; the row's identity-space, collision, retry and reader obligations are
//! proven at the largest scale the default suite can carry.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use beamcraft_cli::changeset::{Applied, apply_changeset};
use beamcraft_cli::plate::PlateError;
use beamcraft_host::revision::{CommitPoint, NoFault, RevisionInfo, RevisionStore};
use beamcraft_host::storage::BlobStore;
use beamcraft_json::{Json, Limits};
use beamcraft_model::id::ActorId;
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

const USER_A: &str = "019a7bf0-0000-7000-8000-00000000c501";
const USER_B: &str = "019a7bf0-0000-7000-8000-00000000c502";
const CHANGESET: &str = "019a7bf0-0000-7000-8000-00000000c510";
const TOKEN_A: &str = "c05-token-a-0123456789abcdefghijklmnopqrstuv";
const TOKEN_B: &str = "c05-token-b-0123456789abcdefghijklmnopqrstuv";

/// Workspace `a` uses `eid(n)`, workspace `b` uses `eid(B + n)`, so a collision only
/// happens where a case forces one deliberately.
const B: usize = 1000;

/// The seed plate every published store starts with.
const SEED: usize = 500;

fn eid(n: usize) -> String {
    format!("019a7bf0-0000-7000-8000-{n:012x}")
}

fn store(path: &Path) -> RevisionStore {
    RevisionStore::new(BlobStore::new(path).unwrap())
}

fn text<'a>(json: &'a Json, key: &str) -> &'a str {
    json.get(key).unwrap().string().unwrap()
}

fn num(json: &Json, path: &[&str]) -> f64 {
    path.iter()
        .fold(json, |j, k| j.get(k).unwrap())
        .float()
        .unwrap()
}

/// One published store and two personal workspaces forked from its empty head.
struct Fixture(PathBuf, BTreeMap<&'static str, String>);

impl Fixture {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("c05-tests");
        fs::create_dir_all(&base).unwrap();
        let root = base.join(format!(
            "e2e-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("published")).unwrap();
        fs::create_dir(root.join("workspaces")).unwrap();
        let f = Self(
            root.clone(),
            [
                ("published", root.join("published")),
                ("workspaces", root.join("workspaces")),
                ("a", root.join("workspaces").join("a")),
                ("b", root.join("workspaces").join("b")),
                ("other-published", root.join("other-published")),
            ]
            .into_iter()
            .map(|(name, path)| (name, path.to_str().unwrap().to_owned()))
            .collect(),
        );
        // A workspaces forks a published revision, so the store seeds one first.
        f.create("published", SEED, 500, "Seed");
        for (name, user) in [("a", USER_A), ("b", USER_B)] {
            f.ok(&[
                "create-workspace",
                "--store",
                f.s("published"),
                "--workspace-dir",
                f.s(name),
                "--user-id",
                user,
            ]);
        }
        f
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn s(&self, name: &str) -> &str {
        self.1.get(name).unwrap().as_str()
    }

    /// A workspace directory, the other static key of the fixture layout.
    fn ws(&self, name: &str) -> &str {
        self.s(name)
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

    /// The op identity of the current head, as `resolve-workspace --discard` names it.
    fn head_op(&self, name: &str) -> String {
        store(&self.path(name))
            .head()
            .unwrap()
            .unwrap()
            .manifest()
            .content()
            .op()
            .to_string()
    }

    fn create(&self, workspace: &str, n: usize, width: usize, name: &str) -> Json {
        let width = width.to_string();
        let element = eid(n);
        self.ok(&[
            "create-plate",
            "--store",
            self.ws(workspace),
            "--width",
            &width,
            "--length",
            "300",
            "--thickness",
            "8",
            "--new-id",
            &element,
            "--name",
            name,
        ])
    }

    fn publish(&self, workspace: &str) -> (i32, Json) {
        self.run(&[
            "publish-workspace",
            "--store",
            self.s("published"),
            "--workspace-dir",
            self.ws(workspace),
        ])
    }

    fn rebase(
        &self,
        workspace: &str,
        request: usize,
        accepted: &RevisionInfo,
        draft: &RevisionInfo,
    ) -> (i32, Json) {
        let op = eid(request);
        self.run(&[
            "rebase-workspace",
            "--store",
            self.s("published"),
            "--workspace-dir",
            self.ws(workspace),
            "--op-id",
            &op,
            "--expected-head",
            accepted.revision().as_str(),
            "--expected-draft",
            draft.revision().as_str(),
        ])
    }

    /// Packages the workspace's unsent work; returns the digest and base revision.
    fn package(&self, workspace: &str, changeset_id: Option<&str>) -> (String, String) {
        let mut args = vec![
            "create-changeset",
            "--store",
            self.s("published"),
            "--workspace-dir",
            self.ws(workspace),
        ];
        if let Some(identity) = changeset_id {
            args.extend(["--changeset-id", identity]);
        }
        let reply = self.ok(&args);
        let changeset = reply.get("changeset").unwrap();
        (
            text(changeset, "digest").to_owned(),
            text(changeset, "base_revision").to_owned(),
        )
    }

    /// `apply-changeset` in a fresh process, the way an offline client delivers work.
    fn deliver(&self, digest: &str) -> (i32, Json) {
        self.run(&[
            "apply-changeset",
            "--store",
            self.s("published"),
            "--workspace-dir",
            self.ws("b"),
            "--changeset",
            digest,
            "--actor-id",
            USER_B,
        ])
    }

    /// The same delivery in-process, for a caller that needs the typed result.
    fn deliver_in_process(&self, digest: &str) -> Result<Applied, PlateError> {
        apply_changeset(
            &store(&self.path("published")),
            &store(&self.path("workspaces/b")),
            &self.path("workspaces/b"),
            &digest.parse().unwrap(),
            &ActorId::parse(USER_B).unwrap(),
            &mut NoFault,
        )
    }

    /// The delivery dies at `point` of the store commit, after the durable claim binding;
    /// nothing after that point runs, as in a crash. Returns whether it unwound.
    fn crashed_delivery(&self, digest: &str, point: CommitPoint) -> bool {
        struct Crash;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            apply_changeset(
                &store(&self.path("published")),
                &store(&self.path("workspaces/b")),
                &self.path("workspaces/b"),
                &digest.parse().unwrap(),
                &ActorId::parse(USER_B).unwrap(),
                &mut |reached: CommitPoint| {
                    if reached == point {
                        std::panic::resume_unwind(Box::new(Crash));
                    }
                    Ok(())
                },
            )
        }));
        result.is_err()
    }

    /// Closes an interrupted commit the way the operator does after a crash (U07).
    fn recover(&self) -> (i32, Json) {
        self.run(&[
            "recover-revision",
            "--store",
            self.s("published"),
            "--min-age-secs",
            "0",
        ])
    }

    /// Number of blobs ever written into a store.
    fn blobs(&self, name: &str) -> usize {
        let mut count = 0;
        for shard in fs::read_dir(self.path(name).join("blobs")).unwrap() {
            for blob in fs::read_dir(shard.unwrap().path()).unwrap() {
                blob.unwrap();
                count += 1;
            }
        }
        count
    }

    /// Every plate of a store as (element id, name, width, volume mm3).
    fn plates(&self, name: &str) -> Vec<(String, String, f64, f64)> {
        let inspected = self.ok(&["inspect-elements", "--store", self.s(name)]);
        assert_eq!(text(&inspected, "outcome"), "plate_inspection");
        inspected
            .get("plates")
            .unwrap()
            .array()
            .unwrap()
            .iter()
            .map(|plate| {
                (
                    text(plate, "element_id").to_owned(),
                    text(plate, "name").to_owned(),
                    num(plate, &["dimensions_mm", "width"]),
                    num(plate, &["quantities", "volume", "mm3"]),
                )
            })
            .collect()
    }

    /// Writes a token file readable by its owner only.
    fn token(&self, name: &str, value: &str) -> String {
        let path = self.path(name);
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

    /// Starts a real coordinator over the fixture's published store and workspaces root.
    fn serve(&self, clients: &[(&str, &str)], max_requests: u32) -> Server {
        let max = max_requests.to_string();
        let mut args = vec![
            "serve-coordinator".to_owned(),
            "--store".into(),
            self.s("published").to_owned(),
            "--workspaces-root".into(),
            self.s("workspaces").to_owned(),
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
            address: text(&listening, "address").to_owned(),
        }
    }

    /// The stored changeset payload of the workspace.
    fn workspace_blob(&self, digest: &str) -> PathBuf {
        self.path("workspaces/b")
            .join("blobs")
            .join(&digest[..2])
            .join(digest)
    }

    /// Flips one byte inside the stored changeset payload.
    fn corrupt_workspace_blob(&self, digest: &str) {
        let path = self.workspace_blob(digest);
        let mut bytes = fs::read(&path).unwrap();
        bytes[40] ^= 0xff;
        fs::write(&path, bytes).unwrap();
    }

    /// Restores a corrupted payload from its untouched copy.
    fn rewrite_workspace_blob(&self, digest: &str, bytes: &[u8]) {
        fs::write(self.workspace_blob(digest), bytes).unwrap();
    }

    /// Plants bytes in the workspace store under their true content digest.
    fn plant_workspace_bytes(&self, bytes: &[u8]) -> String {
        let ws = BlobStore::new(self.path("workspaces/b")).unwrap();
        ws.publish(bytes).unwrap().0.to_string()
    }

    /// A second, independent published store with its own project.
    fn second_published(&self) -> PathBuf {
        let dir = self.path("other-published");
        fs::create_dir(&dir).unwrap();
        dir
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.0.is_absolute() && self.0.file_name().is_some());
        let _ = fs::remove_dir_all(&self.0);
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

fn publish_via(
    f: &Fixture,
    address: &str,
    token_file: &str,
    workspace: &str,
    changeset: &str,
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
        changeset,
        "--expected-head",
        expected_head,
    ])
}

fn revision_sequence(store_path: &Path, revision: &str) -> Option<u64> {
    let store = store(store_path);
    (1..=store.head().unwrap().unwrap().sequence())
        .find(|&sequence| store.info_at(sequence).unwrap().revision().as_str() == revision)
}

// C05-01 at suite scale: two workspaces create independent elements against one base, A
// publishes, B rebases without publishing and then publishes; the reopened store holds
// every element exactly once with its authored value, and the final accepted revision
// agrees with the independent reader.
#[test]
fn two_offline_workspaces_publish_independent_elements_and_lose_nothing() {
    const N: usize = 32;
    let f = Fixture::new();
    let mut created: Vec<(String, String, f64, f64)> =
        vec![(eid(SEED), "Seed".to_owned(), 500.0, 1_200_000.0)];
    let author = |workspace: &str, first: usize, width: usize, prefix: &str| {
        (1..=N)
            .map(|n| {
                let reply = f.create(workspace, first + n, width, &format!("{prefix}-{n}"));
                let plate = reply.get("plate").unwrap();
                (
                    text(plate, "element_id").to_owned(),
                    format!("{prefix}-{n}"),
                    width as f64,
                    num(plate, &["quantities", "volume", "mm3"]),
                )
            })
            .collect::<Vec<_>>()
    };
    // The two clients are offline from each other: each workspace is its own store, so their
    // drafts are authored side by side, as two machines would.
    let (a, b) = std::thread::scope(|scope| {
        let b = scope.spawn(|| author("b", B, 600, "B"));
        (author("a", 0, 400, "A"), b.join().unwrap())
    });
    created.extend(a);
    created.extend(b);

    let (code, published) = f.publish("a");
    assert_eq!(code, 0, "{published:?}");
    let accepted = f.head("published");
    let draft = f.head("workspaces/b");
    let (code, rebased) = f.rebase("b", 900, &accepted, &draft);
    assert_eq!(code, 0, "{rebased:?}");
    // A rebase never moves the published head: B still needs an explicit publish.
    assert_eq!(f.head("published").revision(), accepted.revision());
    let (code, moved) = f.publish("b");
    assert_eq!(code, 0, "{moved:?}");

    // The reopened store holds every element exactly once with its authored value.
    let plates = f.plates("published");
    assert_eq!(plates.len(), created.len());
    let mut seen = std::collections::BTreeSet::new();
    for (element, name, width, volume) in &plates {
        assert!(seen.insert(element.clone()), "duplicated element {element}");
        let expected = created
            .iter()
            .find(|(identity, ..)| identity == element)
            .unwrap_or_else(|| panic!("unknown element {element}"));
        assert_eq!(name, &expected.1, "{element}");
        assert_eq!(width, &expected.2, "{element}");
        assert_eq!(volume, &expected.3, "{element}");
    }
    assert_eq!(seen.len(), created.len());
    for (element, _, _, _) in &created {
        assert!(seen.contains(element), "{element} is lost");
    }

    // The final accepted revision opens in the independent reader and agrees plate by
    // plate, so nothing was renamed, dropped or doubled on the way to disk.
    let inspected = f.ok(&["inspect-elements", "--store", f.s("published")]);
    let layers = f.path("layers");
    fs::create_dir(&layers).unwrap();
    for entry in inspected
        .get("revision")
        .unwrap()
        .get("manifest")
        .unwrap()
        .get("layers")
        .unwrap()
        .array()
        .unwrap()
    {
        let digest = text(entry, "digest");
        fs::copy(
            f.path("published")
                .join("blobs")
                .join(&digest[..2])
                .join(digest),
            layers.join(text(entry, "name")),
        )
        .unwrap();
    }
    let expect = created
        .iter()
        .map(|(element, name, width, volume)| {
            format!(
                r#"{{"id":"{element}","name":"{name}","width":{width:?},"length":300.0,"thickness":8.0,"translation":[0.0,0.0,0.0],"cli_volume_mm3":{volume:?}}}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    fs::write(layers.join("expect.json"), format!("[{expect}]")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft-usd-oracle"))
        .arg("revision")
        .arg(layers.join("root.usda"))
        .arg(layers.join("expect.json"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "the independent reader disagreed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = beamcraft_json::decode(
        String::from_utf8_lossy(&output.stdout).trim().as_bytes(),
        &Limits::DEFAULT,
    )
    .unwrap();
    assert_eq!(
        report.get("plates").unwrap().array().unwrap().len(),
        2 * N + 1
    );
}

// C05-02: after a base mismatch the only forward path is rebase, repackage under a new
// changeset identity, publish against the observed head; a retry that skips rebase keeps
// failing with the same typed answer, and every attempt is bounded.
#[test]
fn after_a_base_mismatch_only_rebase_repackage_publish_moves_forward() {
    let f = Fixture::new();
    let fork = f.head("published").revision().as_str().to_owned();
    f.create("a", 1, 400, "A-one");
    f.create("b", B + 1, 600, "B-one");
    let (code, published) = f.publish("a");
    assert_eq!(code, 0, "{published:?}");
    let accepted = f.head("published");

    let (stale, base) = f.package("b", None);
    assert_eq!(base, fork);
    // The refused identity stays bound: every bounded retry answers identically and the
    // head never moves without a rebase.
    for _ in 0..3 {
        let (code, refused) = f.deliver(&stale);
        assert_eq!(code, 2, "{refused:?}");
        assert_eq!(text(&refused, "code"), "BASE_REVISION_MISMATCH");
        assert_eq!(f.head("published").revision(), accepted.revision());
    }

    // B rebases, repackages under a new identity and delivers against the observed head.
    let draft = f.head("workspaces/b");
    let (code, rebased) = f.rebase("b", 901, &accepted, &draft);
    assert_eq!(code, 0, "{rebased:?}");
    let (fresh, rebased_base) = f.package("b", None);
    assert_ne!(fresh, stale, "a rebased changeset needs a new identity");
    assert_eq!(rebased_base, accepted.revision().as_str());
    let (code, applied) = f.deliver(&fresh);
    assert_eq!(code, 0, "{applied:?}");
    assert_eq!(text(&applied, "outcome"), "changeset_applied");

    let plates = f.plates("published");
    assert_eq!(plates.len(), 3);
    assert!(
        plates
            .iter()
            .any(|(element, _, width, _)| element == &eid(1) && width == &400.0)
    );
    assert!(
        plates
            .iter()
            .any(|(element, _, width, _)| element == &eid(B + 1) && width == &600.0)
    );
}

// C05-03 and C05-05: the same changeset identity publishes exactly once across every
// commit crash window, one test per window. The delivery is injected-dead at the point,
// recovery closes the interrupted commit, a fresh attempt finishes it exactly once, later
// submissions replay whatever the current head is, and a different payload under the
// identity is refused.
fn a_changeset_identity_publishes_exactly_once_after_a_crash_at(point: CommitPoint) {
    let f = Fixture::new();
    // B's changeset names the head it was packaged against, so the crash windows run
    // before anyone else publishes.
    f.create("b", B + 1, 600, "B-one");
    let (digest, _) = f.package("b", Some(CHANGESET));

    // The apply dies at `point`, after the durable claim binding.
    assert!(
        f.crashed_delivery(&digest, point),
        "{point:?}: the injected crash did not happen"
    );
    let (code, recovered) = f.recover();
    assert_eq!(code, 0, "{point:?}: {recovered:?}");

    // A fresh process finishes the publication exactly once: a crash before the root
    // switch is finished by the retry, a crash after it is answered from the record.
    let finishes = !matches!(point, CommitPoint::Switched | CommitPoint::Recorded);
    let applied = f.deliver_in_process(&digest).unwrap();
    assert_eq!(applied.replayed, !finishes, "{point:?}");
    // No lost writes and no double publication, whatever the window was.
    let plates = f.plates("published");
    assert_eq!(plates.len(), 2, "{point:?}");
    assert!(
        plates
            .iter()
            .any(|(element, _, width, _)| element == &eid(B + 1) && width == &600.0),
        "{point:?}: B's element is lost"
    );

    // Every submission replays the recorded answer whatever the current head is.
    // A publishes next: its base moved too, so it rebases before its own publication.
    f.create("a", 1, 400, "A-one");
    let accepted = f.head("published");
    let a_draft = f.head("workspaces/a");
    let (code, a_rebased) = f.rebase("a", 907, &accepted, &a_draft);
    assert_eq!(code, 0, "{point:?}: {a_rebased:?}");
    let (code, published) = f.publish("a");
    assert_eq!(code, 0, "{published:?}");
    let head = f.head("published");
    let later = f.deliver_in_process(&digest).unwrap();
    assert!(later.replayed, "{point:?}");
    assert_eq!(f.head("published").revision(), head.revision());

    // A different payload under that identity is refused forever; packaging it needs
    // B's own rebase onto the head that moved meanwhile.
    f.create("b", B + 2, 700, "B-two");
    let accepted = f.head("published");
    let draft = f.head("workspaces/b");
    let (code, rebased) = f.rebase("b", 906, &accepted, &draft);
    assert_eq!(code, 0, "{point:?}: {rebased:?}");
    let (other, _) = f.package("b", Some(CHANGESET));
    assert_ne!(other, digest);
    let (code, reused) = f.deliver(&other);
    assert_eq!(code, 2, "{reused:?}");
    assert_eq!(text(&reused, "code"), "OP_ID_REUSE");
    assert_eq!(f.head("published").revision(), head.revision());
}

#[test]
fn a_changeset_identity_publishes_exactly_once_after_a_crash_at_checked() {
    a_changeset_identity_publishes_exactly_once_after_a_crash_at(CommitPoint::Checked);
}

#[test]
fn a_changeset_identity_publishes_exactly_once_after_a_crash_at_intent_recorded() {
    a_changeset_identity_publishes_exactly_once_after_a_crash_at(CommitPoint::IntentRecorded);
}

#[test]
fn a_changeset_identity_publishes_exactly_once_after_a_crash_at_blobs_verified() {
    a_changeset_identity_publishes_exactly_once_after_a_crash_at(CommitPoint::BlobsVerified);
}

#[test]
fn a_changeset_identity_publishes_exactly_once_after_a_crash_at_manifest_stored() {
    a_changeset_identity_publishes_exactly_once_after_a_crash_at(CommitPoint::ManifestStored);
}

#[test]
fn a_changeset_identity_publishes_exactly_once_after_a_crash_at_record_staged() {
    a_changeset_identity_publishes_exactly_once_after_a_crash_at(CommitPoint::RecordStaged);
}

#[test]
fn a_changeset_identity_publishes_exactly_once_after_a_crash_at_switched() {
    a_changeset_identity_publishes_exactly_once_after_a_crash_at(CommitPoint::Switched);
}

#[test]
fn a_changeset_identity_publishes_exactly_once_after_a_crash_at_recorded() {
    a_changeset_identity_publishes_exactly_once_after_a_crash_at(CommitPoint::Recorded);
}

// C05-04: A and B both fail and retry through a real coordinator; per base exactly one
// publication is admitted, the losers keep their drafts, the accepted order is total and
// matches the queue order, and no accepted head is overwritten.
#[test]
fn interleaved_publications_with_retries_admit_one_per_base_and_lose_nothing() {
    let f = Fixture::new();
    f.create("a", 1, 400, "A-one");
    f.create("b", B + 1, 600, "B-one");
    let base = f.head("published").revision().as_str().to_owned();
    let (cs_a, base_a) = f.package("a", None);
    let (cs_b, base_b) = f.package("b", None);
    assert_eq!(base_a, base);
    assert_eq!(base_b, base);

    let token_a = f.token("token-a", TOKEN_A);
    let token_b = f.token("token-b", TOKEN_B);
    let server = f.serve(&[(USER_A, &token_a), (USER_B, &token_b)], 8);
    let address = server.address.clone();

    let (code, first) = publish_via(&f, &address, &token_a, "a", &cs_a, &base);
    assert_eq!(code, 0, "{first:?}");
    let a_revision = text(&first, "revision_id").to_owned();
    let (code, second) = publish_via(&f, &address, &token_b, "b", &cs_b, &base);
    assert_eq!(code, 2, "{second:?}");
    assert_eq!(text(&second, "outcome"), "rejected");
    assert_eq!(text(&second, "code"), "BASE_REVISION_MISMATCH");
    assert_eq!(text(&second, "current_head"), a_revision);

    // The bounded retry keeps failing with the same typed answer; B's data stays in its
    // draft and never reaches the published store by itself.
    for _ in 0..2 {
        let (code, retried) = publish_via(&f, &address, &token_b, "b", &cs_b, &base);
        assert_eq!(code, 2, "{retried:?}");
        assert_eq!(text(&retried, "outcome"), "rejected");
        assert_eq!(text(&retried, "code"), "BASE_REVISION_MISMATCH");
        assert_eq!(text(&retried, "current_head"), a_revision);
    }
    assert!(
        !f.plates("published")
            .iter()
            .any(|(element, ..)| element == &eid(B + 1))
    );
    assert_eq!(f.plates("b").len(), 2); // the forked seed and B's own element

    // The loser rebases, repackages under a new identity and publishes in its turn.
    let accepted = f.head("published");
    let draft = f.head("workspaces/b");
    let (code, rebased) = f.rebase("b", 901, &accepted, &draft);
    assert_eq!(code, 0, "{rebased:?}");
    let (cs_b2, rebased_base) = f.package("b", None);
    assert_eq!(rebased_base, a_revision);
    let (code, moved) = publish_via(&f, &address, &token_b, "b", &cs_b2, &a_revision);
    assert_eq!(code, 0, "{moved:?}");
    let b_revision = text(&moved, "revision_id").to_owned();

    // Both fail once more on the next base and retry concurrently: the single queue and
    // the expected-base check admit exactly one, and the loser's answer names the winner.
    f.create("a", 2, 450, "A-two");
    f.create("b", B + 2, 650, "B-two");
    // A's accepted base still names its own publication, so it rebases onto B's head
    // before packaging; B's base already names that head.
    let a_accepted = f.head("published");
    let a_draft = f.head("workspaces/a");
    let (code, a_rebased) = f.rebase("a", 907, &a_accepted, &a_draft);
    assert_eq!(code, 0, "{a_rebased:?}");
    let (cs_a2, a_base) = f.package("a", None);
    let (cs_b2, b_base) = f.package("b", None);
    assert_eq!(a_base, b_revision);
    assert_eq!(b_base, b_revision);
    let (winner, loser) = std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            (
                "a",
                &token_a,
                publish_via(&f, &address, &token_a, "a", &cs_a2, &b_revision),
            )
        });
        let b = scope.spawn(|| {
            (
                "b",
                &token_b,
                publish_via(&f, &address, &token_b, "b", &cs_b2, &b_revision),
            )
        });
        let (ra, rb) = (a.join().unwrap(), b.join().unwrap());
        if ra.2.0 == 0 {
            (ra, rb)
        } else {
            assert_eq!(rb.2.0, 0, "{rb:?}");
            (rb, ra)
        }
    });
    let (_winner_workspace, _winner_token, (win_code, win)) = winner;
    let (loser_workspace, loser_token, (lose_code, lose)) = loser;
    assert_eq!(win_code, 0, "{win:?}");
    let winner_revision = text(&win, "revision_id").to_owned();
    assert_eq!(lose_code, 2, "{lose:?}");
    assert_eq!(text(&lose, "outcome"), "rejected");
    assert_eq!(text(&lose, "code"), "BASE_REVISION_MISMATCH");
    assert_eq!(text(&lose, "current_head"), winner_revision);

    // The loser finishes exactly like the first round: rebase, repackage, publish. Its
    // draft head is read fresh — workspace "a" rebased earlier in this round.
    let accepted = f.head("published");
    let draft = f.head(&format!("workspaces/{loser_workspace}"));
    let (code, rebased) = f.rebase(loser_workspace, 902, &accepted, &draft);
    assert_eq!(code, 0, "{rebased:?}");
    let (cs_loser, loser_base) = f.package(loser_workspace, None);
    assert_eq!(loser_base, winner_revision);
    let (code, last) = publish_via(
        &f,
        &address,
        loser_token,
        loser_workspace,
        &cs_loser,
        &winner_revision,
    );
    assert_eq!(code, 0, "{last:?}");
    let last_revision = text(&last, "revision_id").to_owned();

    let (code, stopped) = server.finish();
    assert_eq!(code, 0, "{stopped:?}");
    assert_eq!(text(&stopped, "outcome"), "stopped");
    assert_eq!(f.head("published").revision().as_str(), last_revision);

    // The accepted order is total: sequence numbers follow the queue, no accepted head
    // was overwritten, and every element arrived exactly once with its authored value.
    let published = store(&f.path("published"));
    let order = [
        base.as_str(),
        a_revision.as_str(),
        b_revision.as_str(),
        winner_revision.as_str(),
        last_revision.as_str(),
    ];
    for (offset, revision) in order.iter().enumerate() {
        assert_eq!(
            published
                .info_at(offset as u64 + 1)
                .unwrap()
                .revision()
                .as_str(),
            *revision,
            "accepted order at sequence {}",
            offset + 1
        );
    }
    let plates = f.plates("published");
    assert_eq!(plates.len(), 5);
    for (element, _, width, _) in &plates {
        let expected = match element.as_str() {
            e if e == eid(SEED) => 500.0,
            e if e == eid(1) => 400.0,
            e if e == eid(B + 1) => 600.0,
            e if e == eid(2) => 450.0,
            e if e == eid(B + 2) => 650.0,
            other => panic!("unknown element {other}"),
        };
        assert_eq!(width, &expected, "{element}");
    }
}

// C05-06: two workspaces forced onto one element identity collide; the rebase refuses
// with the typed conflict, nothing is merged, and an explicit discard recovers without
// touching the other actor's accepted edit.
#[test]
fn forced_id_collisions_refuse_and_merge_nothing() {
    let f = Fixture::new();
    f.create("a", 10, 400, "A-collision");
    // B is forced onto the same element identity with a different shape.
    f.create("b", 10, 600, "B-collision");
    let (code, published) = f.publish("a");
    assert_eq!(code, 0, "{published:?}");
    let accepted = f.head("published");
    let blobs_before = f.blobs("published");

    let (code, mismatch) = f.publish("b");
    assert_eq!(code, 2, "{mismatch:?}");
    assert_eq!(text(&mismatch, "code"), "BASE_REVISION_MISMATCH");

    let draft = f.head("workspaces/b");
    let (code, conflicted) = f.rebase("b", 903, &accepted, &draft);
    assert_eq!(code, 2, "{conflicted:?}");
    assert_eq!(text(&conflicted, "code"), "DEPENDENCY_CONFLICT");
    let diagnostics = conflicted.get("diagnostics").unwrap().array().unwrap();
    // Creating the same element in both drafts intersects every group of that target,
    // lifecycle included; the planner reports the full write-write intersection.
    assert!(!diagnostics.is_empty());
    for diagnostic in diagnostics {
        assert_eq!(text(diagnostic, "target"), eid(10));
        assert!(
            matches!(text(diagnostic, "kind"), "write_write" | "lifecycle"),
            "{diagnostic:?}"
        );
        assert_eq!(text(diagnostic, "draft_op"), f.head_op("workspaces/b"));
        assert_eq!(
            text(diagnostic, "accepted_op"),
            accepted.manifest().content().op().to_string()
        );
    }
    assert!(
        diagnostics.iter().any(|d| text(d, "kind") == "write_write"),
        "{diagnostics:?}"
    );

    // Nothing was merged: the published store is what A published, byte for byte.
    assert_eq!(f.head("published").revision(), accepted.revision());
    assert_eq!(f.blobs("published"), blobs_before);
    assert_eq!(f.plates("published").len(), 2);

    // B recovers by renouncing its own colliding operation, re-editing afresh, rebasing
    // and publishing; A's accepted value survives verbatim and B's replacement arrives.
    let colliding = f.head_op("workspaces/b");
    let expected = f.head("workspaces/b");
    let (code, resolved) = f.run(&[
        "resolve-workspace",
        "--workspace-dir",
        f.ws("b"),
        "--op-id",
        &eid(904),
        "--discard",
        &colliding,
        "--expected-draft",
        expected.revision().as_str(),
    ]);
    assert_eq!(code, 0, "{resolved:?}");
    assert_eq!(text(&resolved, "outcome"), "workspace_resolved");
    f.create("b", B + 3, 600, "B-fresh");
    let accepted = f.head("published");
    let draft = f.head("workspaces/b");
    let (code, rebased) = f.rebase("b", 905, &accepted, &draft);
    assert_eq!(code, 0, "{rebased:?}");
    let (code, moved) = f.publish("b");
    assert_eq!(code, 0, "{moved:?}");
    let plates = f.plates("published");
    assert_eq!(plates.len(), 3);
    let collided = plates
        .iter()
        .find(|(element, ..)| element == &eid(10))
        .unwrap();
    assert_eq!(collided.1, "A-collision");
    assert_eq!(
        collided.2, 400.0,
        "A's accepted shape must survive verbatim"
    );
    assert!(
        plates
            .iter()
            .any(|(element, name, width, _)| element == &eid(B + 3)
                && name == "B-fresh"
                && width == &600.0)
    );
}

// C05-07: bounded failure surface of the harness — a corrupt payload, an oversized one
// beyond the decode cap, a future payload version and another project's changeset are
// all typed refusals that write nothing.
#[test]
fn bounded_failures_refuse_without_writing() {
    let f = Fixture::new();
    f.create("a", 1, 400, "A-one");
    f.create("b", B + 1, 600, "B-one");
    let (digest, _) = f.package("b", None);
    let head = f.head("published");
    let blobs_before = f.blobs("published");

    // A corrupted payload is refused; nothing is published.
    let untouched = fs::read(f.workspace_blob(&digest)).unwrap();
    f.corrupt_workspace_blob(&digest);
    let (code, refused) = f.deliver(&digest);
    assert_eq!(code, 2, "{refused:?}");
    assert_eq!(text(&refused, "code"), "INTEGRITY_ERROR");
    assert_eq!(f.head("published").revision(), head.revision());
    assert_eq!(f.blobs("published"), blobs_before);
    f.rewrite_workspace_blob(&digest, &untouched);

    // A payload beyond the decode cap is refused before anything is read into a model.
    let oversized = f.plant_workspace_bytes(&[b'x'; 1_048_577]);
    let (code, too_big) = f.deliver(&oversized);
    assert_eq!(code, 2, "{too_big:?}");
    assert_eq!(text(&too_big, "code"), "LIMIT_EXCEEDED");
    assert_eq!(f.head("published").revision(), head.revision());

    // A future payload version is refused, never guessed.
    let mut future = untouched.clone();
    future[0] = future[0].wrapping_add(1);
    let planted = f.plant_workspace_bytes(&future);
    let (code, version) = f.deliver(&planted);
    assert_eq!(code, 2, "{version:?}");
    assert_eq!(text(&version, "code"), "UNSUPPORTED_VERSION");
    assert_eq!(f.head("published").revision(), head.revision());
    assert_eq!(f.blobs("published"), blobs_before);

    // Another project's changeset never applies: the receiver checks the project first
    // and leaves the other store untouched.
    let other = f.second_published();
    f.ok(&[
        "create-plate",
        "--store",
        other.to_str().unwrap(),
        "--width",
        "400",
        "--length",
        "300",
        "--thickness",
        "8",
        "--new-id",
        &eid(20),
    ]);
    let other_head = store(&other).head().unwrap().unwrap();
    let (code, wrong_project) = f.run(&[
        "apply-changeset",
        "--store",
        other.to_str().unwrap(),
        "--workspace-dir",
        f.ws("b"),
        "--changeset",
        &digest,
        "--actor-id",
        USER_B,
    ]);
    assert_eq!(code, 2, "{wrong_project:?}");
    assert_eq!(text(&wrong_project, "code"), "INVALID_ARGUMENT");
    assert_eq!(
        store(&other).head().unwrap().unwrap().revision(),
        other_head.revision()
    );
    assert_eq!(
        revision_sequence(&other, other_head.revision().as_str()),
        Some(1)
    );
}
