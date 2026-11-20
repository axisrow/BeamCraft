// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! `run-script`: one `beamcraft` process holding one opened store, executing the commands
//! of one JSON script file in order and answering one JSON summary (`#738`). The commands
//! are not a new interpreter: every entry dispatches through the same registry routes and
//! session machinery as the one-process CLI and a `session` line, so the per-command
//! refusals and JSON shapes are identical, and the batch words work inside a script the
//! way they work inside a session (a refusal abandons the batch whole). The mode is the
//! file-transport sibling of `session`: where the session reads stdin lines, `run-script`
//! reads the whole script up front, so a driver that cannot interleave pipe writes and
//! reads (a test harness, a queued build step) amortizes the process start, the store open
//! and the revision-chain verification of many commands into one invocation.

use beamcraft_host::revision::RevisionStore;
use beamcraft_host::storage::BlobStore;
use beamcraft_json::{Json, Limits};
use session::Refusal;
use session::draft;
use std::collections::BTreeSet;
use std::ffi::OsString;

/// Longest script file `run-script` reads; larger input is refused, never truncated.
const MAX_SCRIPT_BYTES: usize = 32 * 1024 * 1024;

/// The two options `run-script` itself takes.
const SCRIPT_OPTIONS: &[&str] = &["--store", "--file"];

/// Runs the script to the end. The process exit is 0 for a script that executed (a
/// command-level refusal is an entry of the summary, not a process status, as in a
/// session); every script-level problem — options, file, JSON shape, store, lease — is a
/// JSON refusal with exit 2.
pub fn execute(args: &[OsString]) -> std::process::ExitCode {
    let options = match script_options(args.get(1..).unwrap_or_default()) {
        Ok(options) => options,
        Err(refusal) => return print_refusal(&refusal),
    };
    // A serving script holds its store for as long as it runs, so a compaction cannot
    // start under it (U14) — the shared lease a compaction respects, under the
    // run-script word itself.
    let lease = match crate::history_cmd::script_lease(args) {
        Ok(lease) => lease,
        Err((json, _)) => {
            let _ = crate::session_cmd::print_json(&json);
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
    let script = match read_script(&options.file) {
        Ok(script) => script,
        Err(refusal) => return print_refusal(&refusal),
    };
    // Bound by name: `let _` would drop the lease (and the lock) right here.
    let _lease = lease;
    let mut session = crate::session_cmd::Session {
        store: &store,
        root: options.store,
        batch_ops: BTreeSet::new(),
        guard: None,
    };
    let mut replies = Vec::new();
    let mut failed = 0u64;
    let mut total_commands;
    for tokens in script {
        for (json, exit) in session.run_line(tokens) {
            if exit != 0 {
                failed += 1;
            }
            replies.push(Json::object([
                ("exit", Json::uint(u64::from(exit))),
                ("reply", json),
            ]));
        }
    }
    total_commands = replies.len() as u64;
    // A script that ends with an open batch abandons it, exactly as a session's EOF does:
    // nothing was on disk, so there is nothing to recover and the summary says so.
    if let Some(commands) = draft::take().map(|state| state.commands.len()) {
        session.batch_ops.clear();
        session.guard = None;
        total_commands += 1;
        replies.push(Json::object([
            ("exit", Json::uint(0)),
            (
                "reply",
                Json::object([
                    ("outcome", Json::text("batch_abandoned")),
                    ("reason", Json::text("the script ended with an open batch")),
                    ("commands", Json::uint(commands as u64)),
                ]),
            ),
        ]));
    }
    // The envelope is one reply like any other and stays inside the output budget the
    // way `check_cmd::insert_checks` shrinks a reply that grew too big. Everything has
    // committed by now, so a bare LIMIT_EXCEEDED here would be indistinguishable from a
    // script refused before running and would send a retry into duplicate-id refusals:
    // the summary stays complete, the oldest replies stay, and the newest are counted in
    // `replies_omitted`.
    let mut kept = replies;
    let mut omitted = 0u64;
    let mut envelope = script_envelope(&kept, total_commands, failed, omitted);
    while envelope.encode(&Limits::DEFAULT).is_err() {
        match kept.pop() {
            Some(_) => {
                omitted += 1;
                envelope = script_envelope(&kept, total_commands, failed, omitted);
            }
            None => break,
        }
    }
    if crate::session_cmd::print_json(&envelope).is_err() {
        return std::process::ExitCode::from(3);
    }
    std::process::ExitCode::SUCCESS
}

/// The whole-script summary envelope over the replies kept so far.
fn script_envelope(replies: &[Json], total: u64, failed: u64, omitted: u64) -> Json {
    let mut fields = Json::Object(Default::default());
    if let Json::Object(entries) = &mut fields {
        entries.insert("outcome".to_owned(), Json::text("script_complete"));
        entries.insert("commands".to_owned(), Json::uint(total));
        entries.insert("failed".to_owned(), Json::uint(failed));
        if omitted != 0 {
            entries.insert("replies_omitted".to_owned(), Json::uint(omitted));
        }
        entries.insert("replies".to_owned(), Json::Array(replies.to_vec()));
    }
    fields
}

/// The parsed options of one `run-script` invocation.
struct ScriptOptions {
    store: String,
    file: String,
}

fn script_options(args: &[OsString]) -> Result<ScriptOptions, Refusal> {
    let (pairs, remainder) = args.as_chunks::<2>();
    let mut store = None;
    let mut file = None;
    for [key, value] in pairs {
        let key = key
            .to_str()
            .ok_or_else(|| invalid_script("options are not text"))?;
        let value = value
            .to_str()
            .ok_or_else(|| invalid_script("options are not text"))?;
        if !SCRIPT_OPTIONS.contains(&key) {
            return Err(invalid_script(
                "unknown run-script option; run-script takes only --store and --file",
            ));
        }
        let slot = if key == "--store" {
            &mut store
        } else {
            &mut file
        };
        if slot.replace(value.to_owned()).is_some() {
            return Err(invalid_script("an option is given twice"));
        }
    }
    if !remainder.is_empty() {
        return Err(invalid_script("a value is missing"));
    }
    let store = store.ok_or_else(|| invalid_script("--store is required"))?;
    let file = file.ok_or_else(|| invalid_script("--file is required"))?;
    Ok(ScriptOptions { store, file })
}

/// Reads and shapes the script: a JSON array of command lines, each an array of the
/// option tokens the one-process CLI would take (first token the command word). Token
/// values arrive exactly as authored — there is no quoting layer.
fn read_script(path: &str) -> Result<Vec<Vec<String>>, Refusal> {
    let bytes = beamcraft_commands::source_file::read_file(path, MAX_SCRIPT_BYTES)
        .map_err(|e| Refusal::new(e, "the script file could not be read"))?;
    let json = beamcraft_json::decode(&bytes, &Limits::DEFAULT)
        .map_err(|_| invalid_script("the script file is not a JSON document within the bounds"))?;
    let entries = json
        .array()
        .map_err(|_| invalid_script("the script is a JSON array of command lines"))?;
    entries
        .iter()
        .map(|entry| {
            let items = entry
                .array()
                .map_err(|_| invalid_script("a script entry is an array of option tokens"))?;
            let mut tokens = Vec::with_capacity(items.len());
            for item in items {
                tokens.push(
                    item.string()
                        .map(str::to_owned)
                        .map_err(|_| invalid_script("a script entry token is not a string"))?,
                );
            }
            if tokens.is_empty() {
                return Err(invalid_script("a script entry names no command"));
            }
            Ok(tokens)
        })
        .collect()
}

fn invalid_script(message: &'static str) -> Refusal {
    Refusal::new(beamcraft_model::error::ErrorCode::InvalidArgument, message)
}

fn print_refusal(refusal: &Refusal) -> std::process::ExitCode {
    if crate::session_cmd::print_json(&crate::session_cmd::refusal_json(refusal)).is_err() {
        return std::process::ExitCode::from(3);
    }
    std::process::ExitCode::from(2)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

    use super::*;

    fn script(json: &str) -> Result<Vec<Vec<String>>, Refusal> {
        let path = std::env::temp_dir().join(format!(
            "beamcraft-script-parse-{}-{}.json",
            std::process::id(),
            json.len()
        ));
        std::fs::write(&path, json).unwrap();
        let parsed = read_script(path.to_str().unwrap());
        let _ = std::fs::remove_file(&path);
        parsed
    }

    #[test]
    fn a_script_is_an_array_of_token_arrays() {
        let parsed = script(r#"[["define-material","--grade","X"],["open-revision"]]"#).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0], vec!["define-material", "--grade", "X"]);
        assert_eq!(parsed[1], vec!["open-revision"]);
    }

    #[test]
    fn a_script_entry_needs_at_least_one_string_token() {
        assert!(script("[]").unwrap().is_empty());
        assert!(script(r#"[["create-plate"]]"#).is_ok());
        assert!(
            script(r#"["create-plate"]"#)
                .unwrap_err()
                .message
                .contains("entry")
        );
        assert!(
            script(r#"[[]]"#)
                .unwrap_err()
                .message
                .contains("no command")
        );
        assert!(
            script(r#"[["a",42]]"#)
                .unwrap_err()
                .message
                .contains("string")
        );
        assert!(script("not json").is_err());
    }

    #[test]
    fn options_take_store_and_file_only() {
        let args: Vec<OsString> = ["--store", "/tmp/s", "--file", "/tmp/c.json"]
            .iter()
            .map(OsString::from)
            .collect();
        let parsed = script_options(&args).unwrap();
        assert_eq!(parsed.store, "/tmp/s");
        assert_eq!(parsed.file, "/tmp/c.json");
        let unknown: Vec<OsString> = ["--store", "/tmp/s", "--tier", "core"]
            .iter()
            .map(OsString::from)
            .collect();
        assert!(script_options(&unknown).is_err());
        let missing: Vec<OsString> = ["--store", "/tmp/s"].iter().map(OsString::from).collect();
        assert!(script_options(&missing).is_err());
        let dangling: Vec<OsString> = ["--store", "/tmp/s", "--file"]
            .iter()
            .map(OsString::from)
            .collect();
        assert!(script_options(&dangling).is_err());
    }

    #[test]
    fn an_oversized_envelope_keeps_the_summary_and_counts_the_dropped_replies() {
        // Review of the 1119 batch replay: an envelope over the budget printed a bare
        // LIMIT_EXCEEDED after every command had committed. The shrink keeps the summary
        // truthful instead: the oldest replies stay and the newest are counted.
        // A reply carries at most a 4 KiB string under the reply budget, so the fat line
        // stays under the per-string cap: the overflow must come from the whole
        // envelope, not from one string that could never encode.
        let fat = Json::text("x".repeat(3900));
        let line = |n: u64| {
            Json::object([
                ("exit", Json::uint(n)),
                ("reply", Json::object([("line", fat.clone())])),
            ])
        };
        let replies: Vec<Json> = (0..512u64).map(line).collect();
        let total = replies.len() as u64;
        let mut kept = replies;
        let mut omitted = 0u64;
        let mut envelope = script_envelope(&kept, total, 0, omitted);
        while envelope.encode(&Limits::DEFAULT).is_err() {
            match kept.pop() {
                Some(_) => {
                    omitted += 1;
                    envelope = script_envelope(&kept, total, 0, omitted);
                }
                None => break,
            }
        }
        assert!(
            envelope.encode(&Limits::DEFAULT).is_ok(),
            "the shrunk envelope fits the budget"
        );
        let entries = match &envelope {
            Json::Object(entries) => entries.clone(),
            _ => Default::default(),
        };
        assert_eq!(entries.get("outcome"), Some(&Json::text("script_complete")));
        assert_eq!(entries.get("commands"), Some(&Json::uint(total)));
        assert_eq!(entries.get("replies_omitted"), Some(&Json::uint(omitted)));
        assert!(omitted > 0, "the fat replies overflowed the budget");
        let kept_replies = match entries.get("replies") {
            Some(Json::Array(list)) => list.clone(),
            _ => Vec::new(),
        };
        assert_eq!(kept_replies.len() as u64 + omitted, total);
        assert!(!kept_replies.is_empty(), "the oldest replies stay");
        assert_eq!(kept_replies[0], line(0));
    }
}
