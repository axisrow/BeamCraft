// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! `session`: one `beamcraft` process holding one opened store, executing one command per
//! stdin line and answering one JSON object per stdout line (`I424`). The mode is strictly
//! opt-in: without `session` the binary behaves exactly as today. There is no socket, no
//! listener and no discovery — the agent starts the process itself and owns its pipes
//! (`AGENTS.md` §3 is satisfied by construction), so the session adds no authentication
//! surface beyond running it.
//!
//! Commands keep the option grammar, the refusals and the JSON shapes of the one-process
//! CLI: every line is dispatched through the same registry routes and adapters. Between
//! `batch-open` and `batch-commit` the modelling commands admit into the in-memory draft
//! (see `session::draft`) and the batch's single commit publishes one revision; a refusal
//! abandons the batch whole, the session stays alive and ready for a new batch, and nothing
//! between the brackets ever touches the store.

use beamcraft_commands::batch::{self, BatchCommit};
use beamcraft_commands::registry::{self, Route};
use beamcraft_host::control::{Cancellation, Control};
use beamcraft_host::ids::IdSource;
use beamcraft_host::revision::RevisionStore;
use beamcraft_host::storage::BlobStore;
use beamcraft_json::{Json, Limits};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{OpId, ProjectId, RevisionId, WorkspaceId};
use session::Refusal;
use session::draft;
use session::pipeline::load_base;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::BufRead;

/// Longest stdin line the session accepts; longer input is refused, never truncated.
const MAX_LINE_BYTES: usize = 1024 * 1024;

/// The one option `session` itself takes.
const SESSION_OPTIONS: &[&str] = &["--store"];

/// Runs `session` until stdin ends. The process exit is 0 for a session that started; every
/// command-level refusal is a JSON line, not a process status.
pub fn execute(args: &[OsString]) -> std::process::ExitCode {
    // The command word itself is not an option pair.
    let options = match session_options(args.get(1..).unwrap_or_default()) {
        Ok(options) => options,
        Err(refusal) => return print_refusal(&refusal),
    };
    // A serving session holds its store for as long as it runs, so a compaction cannot start
    // under it (U14) — the same lease every one-process command holds while it runs.
    let lease = match crate::history_cmd::coordinator_lease(args) {
        Ok(lease) => lease,
        Err((json, _)) => {
            let _ = print_json(&json);
            return std::process::ExitCode::from(2);
        }
    };
    let store = match BlobStore::new(&options.store) {
        Ok(blobs) => RevisionStore::new(blobs),
        Err(e) => {
            let _ = print_refusal(&Refusal::new(e.code(), "the store could not be opened"));
            return std::process::ExitCode::from(2);
        }
    };
    let _ = lease;
    if print_json(&Json::object([("outcome", Json::text("session_ready"))])).is_err() {
        return std::process::ExitCode::from(3);
    }
    let mut session = Session {
        store: &store,
        root: options.store,
        batch_ops: BTreeSet::new(),
        guard: None,
    };
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => return exit_after(),
        };
        if line.len() > MAX_LINE_BYTES {
            let _ = print_refusal(&Refusal::new(
                ErrorCode::LimitExceeded,
                "the command line exceeds the session line budget",
            ));
            continue;
        }
        match tokenize(&line) {
            Ok(tokens) if tokens.is_empty() => continue,
            Ok(tokens) => session.run_line(tokens),
            Err(message) => {
                let _ = print_refusal(&Refusal::invalid(message));
            }
        }
    }
    exit_after()
}

/// A session that ends while a batch is open abandons the batch: nothing was on disk, so
/// there is nothing to recover and the JSON line says so before the process exits.
fn exit_after() -> std::process::ExitCode {
    if let Some(commands) = draft::take().map(|state| state.commands.len()) {
        let _ = print_json(&Json::object([
            ("outcome", Json::text("batch_abandoned")),
            ("reason", Json::text("the session ended with an open batch")),
            ("commands", Json::uint(commands as u64)),
        ]));
    }
    std::process::ExitCode::SUCCESS
}

/// The parsed options of one `session` invocation.
struct SessionOptions {
    store: String,
}

fn session_options(args: &[OsString]) -> Result<SessionOptions, Refusal> {
    let (pairs, remainder) = args.as_chunks::<2>();
    let mut store = None;
    for [key, value] in pairs {
        let key = key
            .to_str()
            .ok_or_else(|| invalid_session("options are not text"))?;
        let value = value
            .to_str()
            .ok_or_else(|| invalid_session("options are not text"))?;
        if !SESSION_OPTIONS.contains(&key) {
            return Err(invalid_session(
                "unknown session option; session takes only --store",
            ));
        }
        if store.replace(value.to_owned()).is_some() {
            return Err(invalid_session("an option is given twice"));
        }
    }
    if !remainder.is_empty() {
        return Err(invalid_session("a value is missing"));
    }
    let store = store.ok_or_else(|| invalid_session("--store is required"))?;
    Ok(SessionOptions { store })
}

/// The running session: the store it holds and the operation ids its open batch carried.
/// The batch guard lives here, so the draft stays open between the bracket lines.
struct Session<'a> {
    store: &'a RevisionStore,
    root: String,
    batch_ops: BTreeSet<String>,
    guard: Option<draft::BatchGuard>,
}

impl<'a> Session<'a> {
    /// Runs one line: a batch word or a command through its own adapter. A command refused
    /// inside an open batch abandons the batch whole after its refusal line, exactly as the
    /// design promises: nothing partial is ever published, and the session stays alive.
    fn run_line(&mut self, tokens: Vec<String>) {
        let word = tokens
            .first()
            .map(|token| token.as_str())
            .unwrap_or_default()
            .to_owned();
        let tail = tokens.get(1..).unwrap_or_default();
        let (json, exit) = match word.as_str() {
            "batch-open" => (self.batch_open(tail), 0u8),
            "batch-commit" => (self.batch_commit(tail), 0u8),
            "batch-abort" => (self.batch_abort(), 0u8),
            _ => self.command(word, tokens),
        };
        let _ = print_json(&json);
        if exit != 0 && draft::is_open() {
            let abandoned = draft::take().map(|state| state.commands.len());
            self.batch_ops.clear();
            self.guard = None;
            let _ = print_json(&Json::object([
                ("outcome", Json::text("batch_abandoned")),
                (
                    "reason",
                    Json::text("a command inside the batch was refused"),
                ),
                ("commands", Json::uint(abandoned.unwrap_or_default() as u64)),
            ]));
        }
    }

    /// Dispatches one command line through the registry: the same routes, refusals and JSON
    /// shapes as the one-process CLI.
    fn command(&mut self, word: String, tokens: Vec<String>) -> (Json, u8) {
        let route = registry::route(&OsString::from(word.as_str()));
        if route.is_some_and(|route| {
            matches!(
                route,
                Route::Session
                    | Route::Batch
                    | Route::Export
                    | Route::ExportView
                    | Route::Coordinator
            )
        }) {
            let refusal = invalid_session("this command cannot run inside a session");
            return (refusal_json(&refusal), refusal_exit(refusal.code));
        }
        if let Err(refusal) = self.check_command(&tokens, route) {
            return (refusal_json(&refusal), refusal_exit(refusal.code));
        }
        let args = tokens.into_iter().map(OsString::from).collect::<Vec<_>>();
        crate::dispatch(route, &args)
    }

    /// The session-level admission rules of one command line: the store must be the session's
    /// own, an operation id may not repeat inside the open batch, and only modelling routes
    /// may run between the brackets (their writes route through the draft, so nothing can
    /// escape a batch by writing the store behind its back).
    fn check_command(&mut self, tokens: &[String], route: Option<Route>) -> Result<(), Refusal> {
        let given_store = option_value(tokens, "--store");
        if given_store.is_some_and(|store| store != self.root) {
            return Err(invalid_session(
                "the command names another store than the session holds",
            ));
        }
        if !draft::is_open() {
            return Ok(());
        }
        match route {
            Some(route) if batch::batchable(route) => {}
            _ => {
                return Err(invalid_session(
                    "only modelling commands may run inside a batch",
                ));
            }
        }
        if let Some(op) = option_value(tokens, "--op-id")
            && !self.batch_ops.insert(op.to_owned())
        {
            return Err(Refusal::new(
                ErrorCode::OpIdReuse,
                "the operation id was already admitted into this batch",
            ));
        }
        Ok(())
    }

    /// `batch-open`: pins the verified head as the base the whole batch builds on and commits
    /// against. A store without a revision is refused: the first revision belongs outside a
    /// batch.
    fn batch_open(&mut self, options: &[String]) -> Json {
        self.batch_open_inner(options)
            .unwrap_or_else(|refusal| refusal_json(&refusal))
    }

    fn batch_open_inner(&mut self, options: &[String]) -> Result<Json, Refusal> {
        batch_word_options(options)?;
        if draft::is_open() {
            return Err(invalid_session("a batch is already open"));
        }
        let head = match self.store.open() {
            Ok(head) => head,
            Err(e) => {
                return Err(Refusal::new(
                    e.code(),
                    "the store has no revision to batch on; create the first revision outside a batch",
                ));
            }
        };
        let sequence = head.info().sequence();
        let mut ids = IdSource::system();
        // The base of a verified head is the head's own; the given ids are only a
        // first-revision default, which a batch never creates.
        let mut fresh = |what: &'static str| ids.next_id().map_err(|_| invalid_session(what));
        let project = fresh("no identity could be generated")?;
        let workspace = fresh("no identity could be generated")?;
        let (_, base) = load_base(
            self.store,
            &ProjectId::from_uuid(project),
            &WorkspaceId::from_uuid(workspace),
        )?;
        let digest = base.digest.to_string();
        // Drop a stale guard first: every path that consumes the batch clears the field, so
        // a guard left here belongs to an already-empty slot and its drop must not run
        // after the new one is installed.
        drop(self.guard.take());
        match draft::hold(base, sequence) {
            Ok(guard) => {
                self.guard = Some(guard);
                Ok(Json::object([
                    ("outcome", Json::text("batch_opened")),
                    ("base", Json::text(digest.as_str())),
                    ("base_sequence", Json::uint(sequence)),
                ]))
            }
            Err(refusal) => Err(refusal),
        }
    }

    /// `batch-commit`: ends the draft and publishes its accumulated state as one revision.
    /// A refusal (a moved head, a budget overrun) fails the batch whole; the session stays
    /// alive and the next commands run against the unchanged store.
    fn batch_commit(&mut self, options: &[String]) -> Json {
        let parsed = match batch_commit_options(options) {
            Ok(parsed) => parsed,
            Err(refusal) => return refusal_json(&refusal),
        };
        if !draft::is_open() {
            return refusal_json(&invalid_session("no draft batch is open"));
        }
        let control = match parsed.control() {
            Ok(control) => control,
            Err(refusal) => return refusal_json(&refusal),
        };
        if control.remaining().is_err() {
            return refusal_json(&Refusal::new(
                ErrorCode::Cancelled,
                "the batch commit is cancelled",
            ));
        }
        let (project, workspace, base_generation) = match draft::with(|state| {
            (
                state.base.project.clone(),
                state.base.workspace.clone(),
                state.base.generation,
            )
        }) {
            Some(fields) => fields,
            None => return refusal_json(&invalid_session("no draft batch is open")),
        };
        let schema = match beamcraft_layer_engine::authored::schema_identity() {
            Ok(schema) => schema,
            Err(e) => return refusal_json(&Refusal::from(e)),
        };
        let revision_id = parsed.revision_id();
        let op = match beamcraft_host::revision::Uuid7::parse(&parsed.op.to_string()) {
            Ok(op) => op,
            Err(_) => return refusal_json(&invalid_session("the operation identity is invalid")),
        };
        match batch::commit_batch(
            BatchCommit {
                store: self.store,
                op,
                revision_id: &revision_id,
                schema,
                project: &project,
                workspace: &workspace,
                base_generation,
            },
            &mut beamcraft_host::revision::NoFault,
        ) {
            Ok(done) => {
                self.batch_ops.clear();
                self.guard = None;
                let outcome = if done.committed.outcome
                    == beamcraft_host::revision::CommitOutcome::Replayed
                {
                    "batch_replayed"
                } else {
                    "batch_committed"
                };
                let mut fields = Json::Object(Default::default());
                if let Json::Object(entries) = &mut fields {
                    entries.insert("outcome".to_owned(), Json::text(outcome));
                    entries.insert("commands".to_owned(), Json::uint(done.commands as u64));
                    entries.insert("op_id".to_owned(), Json::text(parsed.op.to_string()));
                    entries.insert(
                        "revision_id".to_owned(),
                        Json::text(revision_id.to_string()),
                    );
                    entries.insert("revision".to_owned(), revision_object(&done.committed.info));
                }
                fields
            }
            Err(refusal) => {
                // The batch was consumed: commit_batch ends it before it checks anything.
                self.batch_ops.clear();
                self.guard = None;
                refusal_json(&refusal)
            }
        }
    }

    /// `batch-abort`: abandons the open draft. Nothing was published, so there is nothing to
    /// undo and the store keeps the revision the batch was pinned on.
    fn batch_abort(&mut self) -> Json {
        match draft::take() {
            Some(state) => {
                self.batch_ops.clear();
                self.guard = None;
                Json::object([
                    ("outcome", Json::text("batch_abandoned")),
                    ("commands", Json::uint(state.commands.len() as u64)),
                ])
            }
            None => refusal_json(&invalid_session("no draft batch is open")),
        }
    }
}

/// The `--op-id`/`--revision-id` of a `batch-commit` line, with fresh defaults.
struct BatchCommitOptions {
    op: OpId,
    revision: RevisionId,
    timeout_ms: Option<u64>,
    cancel_after_ms: Option<u64>,
}

impl BatchCommitOptions {
    fn revision_id(&self) -> RevisionId {
        self.revision
    }

    /// The control the commit runs under: the same bounds and semantics as the store-level
    /// commands' `--timeout-ms` / `--cancel-after-ms`.
    fn control(&self) -> Result<Control, Refusal> {
        let cancellation = Cancellation::default();
        if let Some(ms) = self.cancel_after_ms {
            if ms == 0 {
                cancellation.cancel();
            } else {
                let token = cancellation.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(ms));
                    token.cancel();
                });
            }
        }
        Control::new(self.timeout_ms, cancellation)
            .map_err(|e| Refusal::new(e.code, "the commit control is invalid"))
    }
}

fn batch_commit_options(options: &[String]) -> Result<BatchCommitOptions, Refusal> {
    let mut ids = IdSource::system();
    let fresh_id = |ids: &mut IdSource, what: &'static str| {
        ids.next_id()
            .map_err(|_| invalid_session(what))
            .map(|id| id.to_string())
    };
    let mut op = OpId::parse(&fresh_id(
        &mut ids,
        "no operation identity could be generated",
    )?)
    .map_err(|_| invalid_session("no operation identity could be generated"))?;
    let mut revision = None;
    let mut timeout_ms = None;
    let mut cancel_after_ms = None;
    let (pairs, remainder) = options.as_chunks::<2>();
    for [key, value] in pairs {
        let value = value.as_str();
        match key.as_str() {
            "--op-id" => {
                op = OpId::parse(value).map_err(|_| invalid_session("--op-id is malformed"))?
            }
            "--revision-id" => {
                revision = Some(
                    RevisionId::parse(value)
                        .map_err(|_| invalid_session("--revision-id is malformed"))?,
                )
            }
            "--timeout-ms" => {
                timeout_ms = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| invalid_session("--timeout-ms is malformed"))?,
                )
            }
            "--cancel-after-ms" => {
                let ms = value
                    .parse::<u64>()
                    .map_err(|_| invalid_session("--cancel-after-ms is malformed"))?;
                if ms > 120000 {
                    return Err(invalid_session("--cancel-after-ms is out of bounds"));
                }
                cancel_after_ms = Some(ms);
            }
            _ => {
                return Err(invalid_session(
                    "unknown batch-commit option; it takes only identity and control options",
                ));
            }
        }
    }
    if !remainder.is_empty() {
        return Err(invalid_session("a value is missing"));
    }
    let revision = match revision {
        Some(revision) => revision,
        None => RevisionId::parse(&fresh_id(
            &mut ids,
            "no revision identity could be generated",
        )?)
        .map_err(|_| invalid_session("no revision identity could be generated"))?,
    };
    Ok(BatchCommitOptions {
        op,
        revision,
        timeout_ms,
        cancel_after_ms,
    })
}

/// `batch-open` and `batch-abort` take no options.
fn batch_word_options(options: &[String]) -> Result<(), Refusal> {
    if !options.is_empty() {
        return Err(invalid_session(
            "unknown option; this batch word takes no options",
        ));
    }
    Ok(())
}

/// The option grammar of the batch words, shared by the session loop and the one-shot
/// path: the one-shot path parses first, so an unlisted option is refused as unknown
/// before its own context refusal.
pub(crate) fn check_batch_options(word: &str, options: &[OsString]) -> Result<(), Refusal> {
    let mut text = Vec::with_capacity(options.len());
    for option in options {
        text.push(
            option
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| invalid_session("options are not text"))?,
        );
    }
    match word {
        "batch-open" | "batch-abort" => batch_word_options(&text),
        _ => batch_commit_options(&text).map(|_| ()),
    }
}

/// The value of one option in a tokenized line, for the session's own checks.
fn option_value<'a>(tokens: &'a [String], name: &str) -> Option<&'a str> {
    tokens
        .iter()
        .zip(tokens.iter().skip(1))
        .find(|(key, _)| key.as_str() == name)
        .map(|(_, value)| value.as_str())
}

fn invalid_session(message: &'static str) -> Refusal {
    Refusal::new(ErrorCode::InvalidArgument, message)
}

/// The process exit status a refusal would carry as a one-process command; the session
/// itself stays alive and the status only decides batch abandonment.
pub(crate) fn refusal_exit(code: ErrorCode) -> u8 {
    match code {
        ErrorCode::OutcomeUnknown => 4,
        ErrorCode::BackendUnavailable => 3,
        _ => 2,
    }
}

/// One JSON object per stdout line: compact, newline-terminated.
fn print_json(json: &Json) -> Result<(), ()> {
    use std::io::Write;
    match json.encode(&Limits::DEFAULT) {
        Ok(bytes) => {
            let mut out = std::io::stdout().lock();
            out.write_all(&bytes)
                .and_then(|()| out.write_all(b"\n"))
                .and_then(|()| out.flush())
                .map_err(|_| ())
        }
        Err(_) => {
            let mut out = std::io::stdout().lock();
            out.write_all(b"{\"outcome\":\"rejected\",\"code\":\"LIMIT_EXCEEDED\"}\n")
                .and_then(|()| out.flush())
                .map_err(|_| ())
        }
    }
}

/// The revision summary as one nested JSON object.
fn revision_object(info: &beamcraft_host::revision::RevisionInfo) -> Json {
    let mut object = Json::Object(Default::default());
    if let Json::Object(entries) = &mut object {
        for (key, value) in crate::revision::revision_fields(info) {
            entries.insert(key.to_owned(), value);
        }
    }
    object
}

fn refusal_json(refusal: &Refusal) -> Json {
    Json::object([
        (
            "outcome",
            Json::text(if refusal.code == ErrorCode::OutcomeUnknown {
                "outcome_unknown"
            } else if refusal.code == ErrorCode::Cancelled {
                "cancelled"
            } else {
                "rejected"
            }),
        ),
        ("code", Json::text(refusal.code.as_str())),
        ("message", Json::text(refusal.message)),
        ("observed_context", Json::Null),
        ("diagnostics", Json::Array(vec![])),
    ])
}

fn print_refusal(refusal: &Refusal) -> std::process::ExitCode {
    if print_json(&refusal_json(refusal)).is_err() {
        return std::process::ExitCode::from(3);
    }
    std::process::ExitCode::from(2)
}

/// Splits one command line into tokens: whitespace-separated, with `'…'` literal and `"…"`
/// quoted values (`\"` and `\\` escape inside double quotes). An unterminated quote is an
/// error, never a silent truncation.
fn tokenize(line: &str) -> Result<Vec<String>, &'static str> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut started = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => {
                if started {
                    tokens.push(std::mem::take(&mut token));
                    started = false;
                }
            }
            '\'' | '"' => {
                started = true;
                let closing = c;
                loop {
                    let Some(escaped) = chars.next() else {
                        return Err("the line ends inside a quoted value");
                    };
                    if escaped == closing {
                        break;
                    }
                    if escaped == '\\' && closing == '"' {
                        match chars.next() {
                            Some(next) => token.push(next),
                            None => return Err("the line ends inside an escape"),
                        }
                    } else {
                        token.push(escaped);
                    }
                }
            }
            _ => {
                started = true;
                token.push(c);
            }
        }
    }
    if started {
        tokens.push(token);
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

    use super::*;

    fn tokens(line: &str) -> Vec<String> {
        tokenize(line).unwrap()
    }

    #[test]
    fn plain_lines_split_on_whitespace() {
        assert_eq!(
            tokens("create-member --store /tmp/s --length 6000"),
            ["create-member", "--store", "/tmp/s", "--length", "6000"]
        );
    }

    #[test]
    fn quoted_values_keep_their_spaces_and_escapes() {
        assert_eq!(
            tokens("--name 'Main Beam' --note \"a \\\"wide\\\" flange\""),
            ["--name", "Main Beam", "--note", "a \"wide\" flange"]
        );
        // Single quotes are literal: no escape processing inside them.
        assert_eq!(tokens("--name 'a\\b'"), ["--name", "a\\b"]);
    }

    #[test]
    fn an_unterminated_quote_is_an_error() {
        assert!(tokenize("--name 'Main").is_err());
        assert!(tokenize("--name \"wide\\").is_err());
    }

    #[test]
    fn empty_lines_are_empty_tokens() {
        assert!(tokens("").is_empty());
        assert!(tokens("   ").is_empty());
    }

    #[test]
    fn option_values_are_found_by_name() {
        let line = tokens("--store /tmp/s --op-id 01900000-0000-7000-8000-00000000000a");
        assert_eq!(
            option_value(&line, "--op-id"),
            Some("01900000-0000-7000-8000-00000000000a")
        );
        assert_eq!(option_value(&line, "--revision-id"), None);
    }
}
