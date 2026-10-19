// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Contributor-original synthetic inputs; real packaged Rust worker processes, no SDK oracle.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_host::{
    dispatch::{self, AuthorizedRead, Command as HostCommand, Dispatcher},
    storage::BlobStore,
    wire,
    worker::{Cancellation, Control, Session, WorkerConfig, executable_digest},
};
use beamcraft_model::{
    ErrorCode,
    digest::{ContentDigest, Sha256},
};
use beamcraft_protocol::{
    FrameHeader, FrameKind, TransportCode as T,
    json::{self, Json},
    message::{self, Context, Identity, envelope},
};
use std::{
    collections::BTreeSet,
    io::Write,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

const ID: &str = "0199e7a0-0000-7000-8000-000000000001";
static NEXT: AtomicU64 = AtomicU64::new(0);
// Cargo builds one immutable worker artifact for this test executable. Compute the expected
// package identity once; Session::spawn still verifies the actual executable on every launch.
static WORKER_DIGEST: OnceLock<ContentDigest> = OnceLock::new();
struct Fixture {
    root: PathBuf,
    store: BlobStore,
    identity: Identity,
    manifest: ContentDigest,
}
impl Fixture {
    fn new(bytes: &[u8]) -> Self {
        // The store rejects links in the root's ancestors; on macOS the temp directory sits
        // behind the /var -> /private/var symlink, so the root is built from the real path.
        let temp = std::env::temp_dir().canonicalize().unwrap();
        let root = temp.join(format!(
            "beamcraft-u04-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let store = BlobStore::new(&root).unwrap();
        let identity = Identity {
            project_id: ID.into(),
            workspace_id: "0199e7a0-0000-7000-8000-000000000002".into(),
            base_revision: Sha256::digest(b"original synthetic revision").unwrap(),
            workspace_generation: 42,
        };
        let (digest, _) = store.publish(bytes).unwrap();
        let manifest = dispatch::read_manifest(
            &identity,
            "root.usda",
            &[("root.usda".into(), digest, bytes.len() as u64)],
        );
        let manifest = store
            .publish(&manifest.encode(&message::read_limits()).unwrap())
            .unwrap()
            .0;
        Self {
            root,
            store,
            identity,
            manifest,
        }
    }
    fn grant(&self) -> AuthorizedRead {
        AuthorizedRead::authorize(
            &self.store,
            self.manifest,
            self.identity.clone(),
            BTreeSet::new(),
        )
        .unwrap()
    }
    fn config(&self) -> WorkerConfig {
        let executable = PathBuf::from(env!("CARGO_BIN_EXE_beamcraft-usd-worker"));
        WorkerConfig {
            executable_digest: *WORKER_DIGEST
                .get_or_init(|| executable_digest(&executable).unwrap()),
            executable,
            store_root: self.root.clone(),
        }
    }
    fn cli(&self, command: &str) -> Command {
        let config = self.config();
        let mut cli = Command::new(env!("CARGO_BIN_EXE_beamcraft"));
        isolated(&mut cli);
        cli.arg(command)
            .args(["--backend", "rust", "--store"])
            .arg(&self.root)
            .arg("--worker")
            .arg(config.executable)
            .args([
                "--worker-digest",
                &config.executable_digest.to_string(),
                "--project-id",
                &self.identity.project_id,
                "--workspace-id",
                &self.identity.workspace_id,
                "--revision",
                &self.identity.base_revision.to_string(),
                "--generation",
                "42",
                "--manifest",
                &self.manifest.to_string(),
            ]);
        cli
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let (Ok(root), Ok(temp)) = (
            self.root.canonicalize(),
            std::env::temp_dir().canonicalize(),
        ) && root.starts_with(temp)
            && root
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("beamcraft-u04-"))
        {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}
fn isolated(command: &mut Command) {
    command.env_clear();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
        if let Some(root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", root);
        }
    }
}
fn control() -> Control {
    // Functional assertions use the public API default, matching the CLI. Explicit deadline
    // tests below keep their independent short/expired budgets.
    Control::new(None, Cancellation::default()).unwrap()
}
fn model() -> Vec<u8> {
    format!("#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\")\ndef Xform \"E_0199e7a0000070008000000000000001\" {{\ncustom string bim:id = \"{ID}\"\ncustom double demo:value = 77.25\ncustom string description = \"inert unknown original metadata\"\n}}\n").into_bytes()
}
fn big_model() -> Vec<u8> {
    let mut text = String::from("#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\")\n");
    for i in 0..240 {
        text.push_str(&format!("def Xform \"P{i}\" {{ custom double demo:value = 77.25\ncustom string future:label = \"synthetic\"\n}}\n"));
    }
    text.into_bytes()
}
fn read(host: &mut Dispatcher, context: Context) -> dispatch::ResultValue {
    host.execute(
        HostCommand::Read {
            context,
            cursor: None,
        },
        &control(),
    )
    .unwrap()
}

#[test]
fn api_cli_real_worker_parity_without_python_sdk_or_search_paths() {
    let fixture = Fixture::new(&model());
    let mut host = Dispatcher::new(fixture.config());
    assert!(host.process_id().is_none());
    let opened = host
        .execute(HostCommand::Open(fixture.grant()), &control())
        .unwrap();
    assert_eq!(opened.body.get("mutations").unwrap(), &Json::Bool(false));
    let context = opened.context.unwrap();
    assert_eq!(context.identity.workspace_generation, 42);
    let page = read(&mut host, context);
    assert_eq!(page.body.get("items").unwrap().array().unwrap().len(), 1);
    let output = fixture.cli("read").output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let cli = json::decode(&output.stdout, &message::read_limits()).unwrap();
    assert_eq!(cli.get("body").unwrap(), &page.body);
    assert!(output.stderr.is_empty());
    let opened = fixture.cli("open").output().unwrap();
    assert!(opened.status.success());
    let opened = json::decode(&opened.stdout, &message::read_limits()).unwrap();
    assert_eq!(
        opened
            .get("body")
            .unwrap()
            .get("prim_count")
            .unwrap()
            .unsigned()
            .unwrap(),
        1
    );
}

#[test]
fn an_array_the_read_protocol_cannot_carry_is_refused_at_open() {
    // 1024 entries cross the host read protocol; 1025 are refused when the worker opens the layer,
    // not accepted and then failed during the read (which would stop the worker).
    for (n, ok) in [(1024usize, true), (1025, false)] {
        let values = std::iter::repeat_n("1", n).collect::<Vec<_>>().join(",");
        let bytes = format!(
            "#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\")\ndef Xform \"P\" {{\ncustom int[] demo:values = [{values}]\n}}\n"
        )
        .into_bytes();
        let fixture = Fixture::new(&bytes);
        let mut host = Dispatcher::new(fixture.config());
        let opened = host.execute(HostCommand::Open(fixture.grant()), &control());
        if ok {
            let context = opened.unwrap().context.unwrap();
            let page = read(&mut host, context);
            assert_eq!(page.body.get("items").unwrap().array().unwrap().len(), 1);
        } else {
            assert_eq!(opened.unwrap_err().code, ErrorCode::LimitExceeded);
        }
    }
}

#[test]
fn corrupt_numeric_plugin_and_digest_faults_are_safe_machine_errors() {
    for (bytes,expected) in [
        (b"this is not a USD layer".to_vec(),ErrorCode::IntegrityError),
        (b"#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\")\ndef Xform \"P\" {custom double demo:value=1e999999999\n}\n".to_vec(),ErrorCode::NonFiniteNumber),
        (b"#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\"\ncustomLayerData={ string beamcraft:plugin=\"unverified\" })\n".to_vec(),ErrorCode::PermissionDenied),
    ] {
        let fixture=Fixture::new(&bytes);
        let mut host=Dispatcher::new(fixture.config());
        let error=host.execute(HostCommand::Open(fixture.grant()),&control()).unwrap_err();
        assert_eq!(error.code,expected,"{error:?}");
        let output=fixture.cli("open").output().unwrap();
        assert_eq!(output.status.code(),Some(2));
        let json=json::decode(&output.stdout,&message::read_limits()).unwrap();
        assert_eq!(json.get("code").unwrap().string().unwrap(),expected.as_str());
        assert!(!String::from_utf8_lossy(&output.stdout).contains(&fixture.root.to_string_lossy().to_string()));
    }
    let fixture = Fixture::new(&model());
    let mut config = fixture.config();
    config.executable_digest = Sha256::digest(b"wrong package").unwrap();
    let mut host = Dispatcher::new(config);
    assert_eq!(
        host.execute(HostCommand::Open(fixture.grant()), &control())
            .unwrap_err()
            .code,
        ErrorCode::IntegrityError
    );
    assert!(host.process_id().is_none());
}

#[test]
fn real_worker_loss_restarts_retries_and_rotates_context_without_resetting_generation() {
    let fixture = Fixture::new(&model());
    let mut host = Dispatcher::new(fixture.config());
    let context = host
        .execute(HostCommand::Open(fixture.grant()), &control())
        .unwrap()
        .context
        .unwrap();
    let pid = host.process_id().unwrap();
    // The next supervision poll runs only after the read request is submitted to its pipe
    // writer, so this kills the real child during the host's in-flight read operation.
    let old_termination = host.termination_handle().unwrap();
    old_termination.request();
    let page = read(&mut host, context.clone());
    assert_ne!(host.process_id().unwrap(), pid);
    let recovered = page.context.unwrap();
    assert_eq!(recovered.identity, context.identity);
    assert_ne!(recovered.stage_token, context.stage_token);
    // Reusing an old supervision handle cannot kill the replacement incarnation.
    old_termination.request();
    read(&mut host, recovered.clone());
    assert_eq!(
        host.execute(
            HostCommand::Read {
                context,
                cursor: None
            },
            &control()
        )
        .unwrap_err()
        .code,
        ErrorCode::StaleWorkspace
    );
}

#[test]
fn paging_close_stale_cursor_and_uncertain_outcomes() {
    let fixture = Fixture::new(&big_model());
    let mut host = Dispatcher::new(fixture.config());
    let first = host
        .execute(HostCommand::Open(fixture.grant()), &control())
        .unwrap()
        .context
        .unwrap();
    let mut cursor = None;
    let mut paths = BTreeSet::new();
    for _ in 0..4 {
        let result = host
            .execute(
                HostCommand::Read {
                    context: first.clone(),
                    cursor,
                },
                &control(),
            )
            .unwrap();
        for prim in result.body.get("items").unwrap().array().unwrap() {
            assert!(paths.insert(prim.get("path").unwrap().string().unwrap().to_string()));
        }
        let next = result.body.get("next").unwrap();
        cursor = if *next == Json::Null {
            None
        } else {
            Some(next.string().unwrap().into())
        };
    }
    assert_eq!(paths.len(), 240);
    assert!(cursor.is_none());
    let second = host
        .execute(HostCommand::Open(fixture.grant()), &control())
        .unwrap()
        .context
        .unwrap();
    assert_eq!(
        host.execute(
            HostCommand::Read {
                context: first,
                cursor: None
            },
            &control()
        )
        .unwrap_err()
        .code,
        ErrorCode::StaleWorkspace
    );
    host.execute(HostCommand::Close(second.clone()), &control())
        .unwrap();
    assert_eq!(
        host.execute(
            HostCommand::Read {
                context: second,
                cursor: None
            },
            &control()
        )
        .unwrap_err()
        .code,
        ErrorCode::StaleWorkspace
    );
    // Ledger uncertainty works with no process and does not start one.
    let mut host = Dispatcher::new(fixture.config());
    for _ in 0..2 {
        assert_eq!(
            host.execute(
                HostCommand::QueryOutcome {
                    identity: fixture.identity.clone(),
                    op_id: ID.into()
                },
                &control()
            )
            .unwrap_err()
            .code,
            ErrorCode::OutcomeUnknown
        );
    }
    assert!(host.process_id().is_none());
    let output = fixture
        .cli("query-outcome")
        .args(["--op-id", ID])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
}

#[test]
fn native_deadline_and_cancel_keep_the_authorized_read_intent() {
    let fixture = Fixture::new(&big_model());
    let mut host = Dispatcher::new(fixture.config());
    let context = host
        .execute(HostCommand::Open(fixture.grant()), &control())
        .unwrap()
        .context
        .unwrap();
    let token = Cancellation::default();
    token.cancel();
    let cancelled = Control::new(None, token).unwrap();
    assert_eq!(
        host.execute(
            HostCommand::Read {
                context: context.clone(),
                cursor: None
            },
            &cancelled
        )
        .unwrap_err()
        .code,
        ErrorCode::Cancelled
    );
    let expired = Control {
        deadline: Instant::now(),
        cancellation: Cancellation::default(),
    };
    let error = host
        .execute(
            HostCommand::Read {
                context: context.clone(),
                cursor: None,
            },
            &expired,
        )
        .unwrap_err();
    assert_eq!(error.transport, Some(T::Deadline));
    assert_eq!(error.exit_code(), 124);
    assert_eq!(
        read(&mut host, context)
            .context
            .unwrap()
            .identity
            .workspace_generation,
        42
    );
    let output = fixture
        .cli("open")
        .args(["--cancel-after-ms", "0"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(130));
    let output = fixture
        .cli("open")
        .args(["--timeout-ms", "1"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(124));
    // A real ready process is present when a long open exceeds its host deadline.
    let mut session = Session::spawn(&fixture.config(), &control()).unwrap();
    let grant = fixture.grant();
    let mut body = fixture.identity.json();
    if let Json::Object(o) = &mut body {
        o.insert(
            "manifest_digest".into(),
            Json::text(grant.grant_digest().to_string()),
        );
    }
    let result = session.call(
        "OpenStage",
        Json::Null,
        body,
        &Control::new(Some(2), Cancellation::default()).unwrap(),
    );
    assert_eq!(result.unwrap_err().transport, Some(T::Deadline));
}

struct Peer {
    child: Child,
    input: std::process::ChildStdin,
    output: std::process::ChildStdout,
    nonce: String,
}
impl Peer {
    fn new(fixture: &Fixture) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_beamcraft-usd-worker"));
        isolated(&mut command);
        let mut child = command
            .arg("--store")
            .arg(&fixture.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let mut peer = Self {
            child,
            input,
            output,
            nonce: "ab".repeat(32),
        };
        peer.send(envelope(
            "hello",
            &peer.nonce,
            [
                ("offered_versions", Json::Array(vec![Json::text("0.1.0")])),
                ("model_contract", Json::text("0.1.0")),
                (
                    "expected_schema_digest",
                    Json::text(message::schema_digest().to_string()),
                ),
                (
                    "required_features",
                    Json::Array(message::FEATURES.into_iter().map(Json::text).collect()),
                ),
                ("host_limits", message::limits_json(message::read_limits())),
            ],
        ));
        assert_eq!(
            peer.recv().get("type").unwrap().string().unwrap(),
            "capabilities"
        );
        peer.send(envelope(
            "ready",
            &peer.nonce,
            [
                ("protocol_version", Json::text("0.1.0")),
                ("model_contract", Json::text("0.1.0")),
                (
                    "schema_package_digest",
                    Json::text(message::schema_digest().to_string()),
                ),
                (
                    "effective_limits",
                    message::limits_json(message::read_limits()),
                ),
            ],
        ));
        assert_eq!(
            peer.recv().get("type").unwrap().string().unwrap(),
            "ready_ack"
        );
        peer
    }
    fn send(&mut self, j: Json) {
        wire::write(&mut self.input, &j, &message::read_limits()).unwrap();
    }
    fn recv(&mut self) -> Json {
        wire::read(&mut self.output, &message::read_limits()).unwrap()
    }
    fn open(&self, fixture: &Fixture, id: u64, timeout: u64) -> Json {
        let mut body = fixture.identity.json();
        if let Json::Object(o) = &mut body {
            o.insert(
                "manifest_digest".into(),
                Json::text(fixture.grant().grant_digest().to_string()),
            );
        }
        envelope(
            "request",
            &self.nonce,
            [
                ("request_id", Json::uint(id)),
                ("method", Json::text("OpenStage")),
                ("timeout_ms", Json::uint(timeout)),
                ("context", Json::Null),
                ("body", body),
            ],
        )
    }
    fn wait_exit(&mut self) {
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            if self.child.try_wait().unwrap().is_some() {
                return;
            }
            assert!(Instant::now() < until, "worker did not terminate");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn actual_pipe_cancel_unknown_plugins_duplicate_ids_and_partial_bundle_crash() {
    let fixture = Fixture::new(&big_model());
    let mut peer = Peer::new(&fixture);
    peer.send(peer.open(&fixture, 1, 10000));
    peer.send(envelope(
        "cancel",
        &peer.nonce,
        [("target_request_id", Json::uint(1))],
    ));
    let result = peer.recv();
    assert_eq!(
        result.get("outcome").unwrap().string().unwrap(),
        "cancelled"
    );
    assert_eq!(result.get("code").unwrap().string().unwrap(), "CANCELLED");
    // A new request after conclusive cancellation reuses the exact retained bytes and generation.
    peer.send(peer.open(&fixture, 2, 10000));
    assert_eq!(
        peer.recv().get("outcome").unwrap().string().unwrap(),
        "stage_opened"
    );
    peer.send(peer.open(&fixture, 2, 10000));
    peer.wait_exit();
    let mut command = Command::new(env!("CARGO_BIN_EXE_beamcraft-usd-worker"));
    isolated(&mut command);
    assert!(
        !command
            .args(["--plugin", "unverified"])
            .status()
            .unwrap()
            .success()
    );
    // Kill a real process while it is assembling a read operation; the host's retained grant
    // remains usable by a new process. This does not claim a deterministic parser crash site.
    let mut peer = Peer::new(&fixture);
    let payload = peer
        .open(&fixture, 1, 10000)
        .encode(&message::read_limits())
        .unwrap();
    let header = FrameHeader::new(
        FrameKind::Control,
        0,
        payload.len() as u32,
        &message::read_limits(),
    )
    .unwrap();
    peer.input.write_all(&header.encode()).unwrap();
    peer.input.write_all(&payload[..8]).unwrap();
    peer.input.flush().unwrap();
    peer.child.kill().unwrap();
    peer.wait_exit();
    let mut host = Dispatcher::new(fixture.config());
    assert_eq!(
        host.execute(HostCommand::Open(fixture.grant()), &control())
            .unwrap()
            .context
            .unwrap()
            .identity
            .workspace_generation,
        42
    );
}

#[test]
fn u05_byte_cli_and_explicit_backend_refusal() {
    let fixture = Fixture::new(&model());
    let path = fixture.root.join("input.usda");
    std::fs::write(&path, model()).unwrap();
    let digest = Sha256::digest(&model()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args(["blob-put", "--store"])
        .arg(&fixture.root)
        .arg("--file")
        .arg(&path)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        json::decode(&output.stdout, &message::read_limits())
            .unwrap()
            .get("digest")
            .unwrap()
            .string()
            .unwrap(),
        digest.to_string()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args(["blob-verify", "--store"])
        .arg(&fixture.root)
        .args(["--digest", &digest.to_string()])
        .output()
        .unwrap();
    assert!(output.status.success());
    let output = fixture.cli("save").output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let output = fixture
        .cli("read")
        .args(["--backend", "sdk"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2)); // Duplicate selector is rejected, never silently changed.
}

#[test]
fn foreign_provenance_and_corrupt_blob_are_verified_at_the_host_boundary() {
    let foreign = b"#usda 1.0\ndef Xform \"Source\" {custom double demo:value=9\n}\n";
    let root=b"#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\")\ndef Xform \"Attached\" (references=@foreign.usda@</Source>) {}\n";
    let mut fixture = Fixture::new(root);
    let root_digest = Sha256::digest(root).unwrap();
    let foreign_digest = fixture.store.publish(foreign).unwrap().0;
    let manifest = dispatch::read_manifest(
        &fixture.identity,
        "root.usda",
        &[
            ("root.usda".into(), root_digest, root.len() as u64),
            ("foreign.usda".into(), foreign_digest, foreign.len() as u64),
        ],
    );
    fixture.manifest = fixture
        .store
        .publish(&manifest.encode(&message::read_limits()).unwrap())
        .unwrap()
        .0;
    let owned = BTreeSet::from(["root.usda".into()]);
    let grant = AuthorizedRead::authorize(
        &fixture.store,
        fixture.manifest,
        fixture.identity.clone(),
        owned.clone(),
    )
    .unwrap();
    let mut host = Dispatcher::new(fixture.config());
    let context = host
        .execute(HostCommand::Open(grant), &control())
        .unwrap()
        .context
        .unwrap();
    let page = read(&mut host, context);
    assert!(
        page.body
            .get("items")
            .unwrap()
            .array()
            .unwrap()
            .iter()
            .flat_map(|p| p.get("sources").unwrap().array().unwrap())
            .any(
                |s| s.get("layer").unwrap().string().unwrap() == "foreign.usda"
                    && !s.get("owned").unwrap().boolean().unwrap()
            )
    );
    assert_eq!(fixture.store.read(&foreign_digest).unwrap(), foreign);
    assert!(
        fixture
            .store
            .read_capped(&foreign_digest, foreign.len() - 1)
            .is_err()
    );
    let name = foreign_digest.to_string();
    let corrupt = fixture.root.join("blobs").join(&name[..2]).join(&name);
    std::fs::write(corrupt, b"corrupt original synthetic bytes").unwrap();
    assert_eq!(
        AuthorizedRead::authorize(
            &fixture.store,
            fixture.manifest,
            fixture.identity.clone(),
            owned
        )
        .err()
        .unwrap()
        .code,
        ErrorCode::IntegrityError
    );
}
