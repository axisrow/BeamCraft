// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! C02 headless commands: `serve-coordinator` starts a local coordinator explicitly (nothing
//! starts it otherwise), `publish-changeset`, `coordinator-compensate`, `coordinator-rebase` and
//! `inspect-coordinator-head` are its clients.
//!
//! `serve-coordinator` prints one `listening` JSON line with the bound loopback address as soon
//! as it accepts connections, then serves until `--max-requests` requests were handled (or until
//! it is stopped) and prints its final JSON object. Tokens are read from files only, never from
//! the command line, where other users could read them.

use beamcraft_cli::coordinator::PublishHandler;
use beamcraft_host::coordinator::{Client, Coordinator, CoordinatorError, Token, call};
use beamcraft_host::revision::RevisionStore;
use beamcraft_host::storage::BlobStore;
use beamcraft_json::{Json, Limits};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ActorId;
use session::Refusal;
use std::ffi::OsString;
use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;

/// A refusal: an F04 code and a fixed message.
struct Failure {
    rule_id: Option<&'static str>,
    code: ErrorCode,
    message: &'static str,
}

impl From<Refusal> for Failure {
    fn from(e: Refusal) -> Self {
        Self {
            rule_id: e.rule_id,
            code: e.code,
            message: e.message,
        }
    }
}

impl From<CoordinatorError> for Failure {
    fn from(e: CoordinatorError) -> Self {
        Self {
            rule_id: None,
            code: e.code,
            message: e.message,
        }
    }
}

type Run<T> = Result<T, Failure>;

fn invalid(message: &'static str) -> Failure {
    Failure {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message,
    }
}

fn exit_for(code: ErrorCode) -> u8 {
    match code {
        ErrorCode::OutcomeUnknown => 4,
        ErrorCode::BackendUnavailable => 3,
        _ => 2,
    }
}

/// Runs one coordinator command and returns the JSON result and the process exit code.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    match run(args) {
        Ok(json) => {
            // A client prints the coordinator's answer; a refusal inside it keeps its exit code.
            let exit = match json.get("code").ok().and_then(|c| c.string().ok()) {
                Some(code) => ErrorCode::ALL
                    .iter()
                    .find(|c| c.as_str() == code)
                    .map_or(2, |c| exit_for(*c)),
                None => 0,
            };
            (json, exit)
        }
        Err(e) => crate::report::report(Err(session::Refusal {
            rule_id: e.rule_id,
            code: e.code,
            message: e.message,
        })),
    }
}

fn run(args: &[OsString]) -> Run<Json> {
    let args: Vec<&str> = args
        .iter()
        .map(|a| {
            a.to_str()
                .filter(|s| s.len() <= 4096)
                .ok_or_else(|| invalid("argument is not bounded UTF-8"))
        })
        .collect::<Run<_>>()?;
    let (command, tail) = args
        .split_first()
        .ok_or_else(|| invalid("missing command"))?;
    let own: &[&str] = match *command {
        "serve-coordinator" => &[
            "--store",
            "--workspaces-root",
            "--client",
            "--port",
            "--max-requests",
        ],
        "publish-changeset" => &[
            "--address",
            "--token-file",
            "--workspace",
            "--changeset",
            "--expected-head",
        ],
        "coordinator-compensate" => &[
            "--address",
            "--token-file",
            "--workspace",
            "--op-id",
            "--target-revision",
            "--expected-head",
        ],
        "coordinator-rebase" => &[
            "--address",
            "--token-file",
            "--workspace",
            "--op-id",
            "--expected-head",
            "--expected-draft",
        ],
        "inspect-coordinator-head" => &["--address", "--token-file"],
        _ => return Err(invalid("unknown command")),
    };
    if !tail.len().is_multiple_of(2) || tail.len() > 64 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut single = std::collections::BTreeMap::new();
    let mut clients = Vec::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        if !own.contains(key) {
            return Err(invalid("unknown option"));
        }
        // `--client` repeats, one per actor; every other option is given once.
        if *key == "--client" {
            clients.push(*value);
        } else if single.insert(*key, *value).is_some() {
            return Err(invalid("repeated option"));
        }
    }
    let given = |key: &str| single.get(key).copied();
    let required = |key: &str, missing: &'static str| given(key).ok_or_else(|| invalid(missing));
    if *command == "serve-coordinator" {
        let clients = clients
            .iter()
            .map(|spec| {
                let (actor, file) = spec
                    .split_once(':')
                    .ok_or_else(|| invalid("--client is ACTOR_UUID:TOKEN_FILE"))?;
                let actor = ActorId::parse(actor).map_err(|e| Failure {
                    rule_id: None,
                    code: e.code(),
                    message: "client actor is not a full canonical UUIDv7",
                })?;
                Ok(Client {
                    actor,
                    token: Token::from_file(Path::new(file))?,
                })
            })
            .collect::<Run<Vec<_>>>()?;
        let port = given("--port").map_or(Ok(0), |p| {
            p.parse::<u16>()
                .map_err(|_| invalid("--port is 0 to 65535"))
        })?;
        let max_requests = given("--max-requests")
            .map(|n| {
                n.parse::<u64>()
                    .map_err(|_| invalid("--max-requests is a whole number"))
            })
            .transpose()?;
        let store_root = required("--store", "--store is required")?;
        let blobs = BlobStore::new(store_root).map_err(|e| Refusal {
            rule_id: None,
            code: e.code(),
            message: "store root is not usable",
        })?;
        let handler = PublishHandler::new(
            RevisionStore::new(blobs),
            Path::new(required(
                "--workspaces-root",
                "--workspaces-root is required",
            )?),
        )?;
        let coordinator = Coordinator::bind(port, clients, max_requests)?;
        let address = coordinator.local_addr()?;
        announce(&Json::object([
            ("outcome", Json::text("listening")),
            ("address", Json::text(address.to_string())),
        ]))?;
        let handled = coordinator.serve(handler)?;
        return Ok(Json::object([
            ("outcome", Json::text("stopped")),
            ("handled", Json::uint(handled)),
        ]));
    }
    let address = required("--address", "--address is required")?
        .parse::<SocketAddr>()
        .map_err(|_| invalid("--address is IP:PORT"))?;
    let token = Token::from_file(Path::new(required(
        "--token-file",
        "--token-file is required",
    )?))?;
    if *command == "inspect-coordinator-head" {
        return Ok(call(address, &token, "head", [])?);
    }
    if *command == "coordinator-compensate" {
        let fields = [
            (
                "workspace",
                required("--workspace", "--workspace is required")?,
            ),
            ("op_id", required("--op-id", "--op-id is required")?),
            (
                "target_revision",
                required("--target-revision", "--target-revision is required")?,
            ),
            (
                "expected_head",
                required("--expected-head", "--expected-head is required")?,
            ),
        ];
        return Ok(call(
            address,
            &token,
            "compensate",
            fields.map(|(k, v)| (k.to_owned(), Json::text(v))),
        )?);
    }
    if *command == "coordinator-rebase" {
        let fields = [
            (
                "workspace",
                required("--workspace", "--workspace is required")?,
            ),
            ("op_id", required("--op-id", "--op-id is required")?),
            (
                "expected_head",
                required("--expected-head", "--expected-head is required")?,
            ),
            (
                "expected_draft",
                required("--expected-draft", "--expected-draft is required")?,
            ),
        ];
        return Ok(call(
            address,
            &token,
            "rebase",
            fields.map(|(k, v)| (k.to_owned(), Json::text(v))),
        )?);
    }
    let fields = [
        (
            "workspace",
            required("--workspace", "--workspace is required")?,
        ),
        (
            "changeset",
            required("--changeset", "--changeset is required")?,
        ),
        (
            "expected_head",
            required("--expected-head", "--expected-head is required")?,
        ),
    ];
    Ok(call(
        address,
        &token,
        "publish",
        fields.map(|(k, v)| (k.to_owned(), Json::text(v))),
    )?)
}

/// Prints one JSON line and flushes it, so a supervisor can read the address at once.
fn announce(json: &Json) -> Run<()> {
    let bytes = json
        .encode(&Limits::DEFAULT)
        .map_err(|_| invalid("announcement cannot be encoded"))?;
    let mut out = std::io::stdout().lock();
    out.write_all(&bytes)
        .and_then(|()| out.write_all(b"\n"))
        .and_then(|()| out.flush())
        .map_err(|_| Failure {
            rule_id: None,
            code: ErrorCode::BackendUnavailable,
            message: "standard output is not writable",
        })
}
