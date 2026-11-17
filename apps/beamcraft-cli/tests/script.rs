// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! `run-script` end to end (`#738`): one real `beamcraft run-script` process executes a
//! JSON script file through the session machinery, so every command answers with the
//! same JSON shape the one-process CLI prints, a refusal is a summary entry rather than
//! a process status, the batch words publish one revision, and a broken script is a
//! typed refusal that touches no store. The store is always checked from separate
//! one-shot processes.

use crate::support::{Scripted, StoreFixture, at, list, text};
use beamcraft_json::{Json, Limits};
use std::fs;
use std::path::Path;
use std::process::Command;

const USER: &str = "019a7bf0-0000-7000-8000-00000000aa01";

fn fixture(name: &str) -> StoreFixture {
    StoreFixture::new(name)
}

/// Runs `file` as it stands when `entries` is empty; otherwise writes the entries as the
/// script file first. Returns exit code and the decoded summary.
fn raw(store: &str, file: &Path, entries: &[&[&str]]) -> (i32, Json) {
    if !entries.is_empty() {
        let lines: Vec<Json> = entries
            .iter()
            .map(|line| Json::Array(line.iter().map(|token| Json::text(*token)).collect()))
            .collect();
        fs::write(file, Json::Array(lines).encode(&Limits::DEFAULT).unwrap()).unwrap();
    }
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args([
            "run-script",
            "--store",
            store,
            "--file",
            file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let summary = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
    (output.status.code().unwrap(), summary)
}

#[test]
fn a_script_answers_each_command_with_the_one_shot_shapes() {
    let f = fixture("script");
    let store = f.root.join("store");
    let replies = f.script(
        &store,
        &[
            Scripted::ok(&[
                "define-material",
                "--store",
                store.to_str().unwrap(),
                "--grade",
                "RS",
                "--density",
                "7850",
            ]),
            Scripted::ok(&["open-revision", "--store", store.to_str().unwrap()]),
        ],
    );
    assert_eq!(text(&replies[0], "outcome"), "definition_created");
    assert_eq!(text(at(&replies[0], &["definition"]), "grade"), "RS");
    // The read answers from the same dispatch tail as the one-process CLI: the definition
    // revision is the head, and a separate one-shot process agrees.
    assert_eq!(text(&replies[1], "sequence"), "1");
    let (code, head) = f.run("open-revision", &[]);
    assert_eq!(code, 0);
    assert_eq!(text(&head, "sequence"), "1");
}

#[test]
fn a_refused_command_is_a_summary_entry_and_the_script_continues() {
    let f = fixture("script-refused");
    let store = f.root.join("store");
    let replies = f.script(
        &store,
        &[
            Scripted::ok(&[
                "define-material",
                "--store",
                store.to_str().unwrap(),
                "--grade",
                "RS",
                "--density",
                "7850",
            ]),
            Scripted::refused(&[
                "attribute-set",
                "--store",
                store.to_str().unwrap(),
                "--element",
                "019a7bf0-0000-7000-8000-00000000aa10",
                "--name",
                "Nope",
                "--value",
                "x",
            ]),
            Scripted::ok(&["open-revision", "--store", store.to_str().unwrap()]),
        ],
    );
    assert_eq!(text(&replies[1], "code"), "NOT_FOUND");
    // The script continued past the refusal: the read saw the definition revision.
    assert_eq!(text(&replies[2], "sequence"), "1");
}

#[test]
fn a_batch_publishes_one_revision_and_the_words_work_in_a_script() {
    let f = fixture("script-batch");
    let store = f.root.join("store");
    let draft = f.root.join("draft");
    let draft_arg = draft.to_str().unwrap().to_owned();
    // The published store authors the profile and forks the workspace; every line names
    // the script's own store.
    f.script(
        &store,
        &[
            Scripted::ok(&[
                "define-profile",
                "--store",
                store.to_str().unwrap(),
                "--designation",
                "RS-RECT",
                "--width",
                "100",
                "--height",
                "50",
            ]),
            Scripted::ok(&[
                "create-workspace",
                "--store",
                store.to_str().unwrap(),
                "--workspace-dir",
                &draft_arg,
                "--user-id",
                USER,
            ]),
        ],
    );
    let plate = |id: &str, name: &str| {
        Scripted::ok(&[
            "create-plate",
            "--store",
            &draft_arg,
            "--width",
            "200",
            "--length",
            "200",
            "--thickness",
            "10",
            "--new-id",
            id,
            "--name",
            name,
        ])
    };
    // The workspace script runs on the workspace store: the two modelled commands ride
    // one batch and the read after it answers from the committed store.
    let replies = f.script(
        &draft,
        &[
            Scripted::ok(&["batch-open"]),
            plate("019a7bf0-0000-7000-8000-00000000ab01", "One"),
            plate("019a7bf0-0000-7000-8000-00000000ab02", "Two"),
            Scripted::ok(&["batch-commit"]),
            Scripted::ok(&["inspect-elements", "--store", &draft_arg]),
        ],
    );
    let committed = &replies[3];
    assert_eq!(text(committed, "outcome"), "batch_committed");
    assert_eq!(at(committed, &["commands"]).unsigned().unwrap(), 2);
    // The fork plus one batch revision: two modelled commands landed as one.
    let (code, draft_head) = StoreFixture::run_in(&draft_arg, "open-revision", &[]);
    assert_eq!(code, 0);
    assert_eq!(text(&draft_head, "sequence"), "2");
    assert_eq!(list(&replies[4], "plates").len(), 2);
}

#[test]
fn a_script_ending_with_an_open_batch_abandons_it() {
    let f = fixture("script-abandoned");
    let store = f.root.join("store");
    let draft = f.root.join("draft");
    let draft_arg = draft.to_str().unwrap().to_owned();
    let store_arg = store.to_str().unwrap().to_owned();
    f.script(
        &store,
        &[
            Scripted::ok(&[
                "define-profile",
                "--store",
                &store_arg,
                "--designation",
                "RS-RECT",
                "--width",
                "100",
                "--height",
                "50",
            ]),
            Scripted::ok(&[
                "create-workspace",
                "--store",
                &store_arg,
                "--workspace-dir",
                &draft_arg,
                "--user-id",
                USER,
            ]),
        ],
    );
    // The workspace script opens a batch and never commits it.
    let (code, summary) = raw(
        &draft_arg,
        &f.root.join("abandon.json"),
        &[
            &["batch-open"],
            &[
                "create-plate",
                "--store",
                &draft_arg,
                "--width",
                "200",
                "--length",
                "200",
                "--thickness",
                "10",
                "--new-id",
                "019a7bf0-0000-7000-8000-00000000ab01",
                "--name",
                "Never",
            ],
        ],
    );
    // The summary carries one extra abandonment entry after the scripted lines, and the
    // process still exits 0: a refusal is an entry, not a process status.
    assert_eq!(code, 0);
    assert_eq!(text(&summary, "outcome"), "script_complete");
    assert_eq!(at(&summary, &["commands"]).unsigned().unwrap(), 3);
    assert_eq!(at(&summary, &["failed"]).unsigned().unwrap(), 0);
    let replies = list(&summary, "replies");
    assert_eq!(
        text(at(&replies[2], &["reply"]), "outcome"),
        "batch_abandoned"
    );
    assert_eq!(
        text(at(&replies[2], &["reply"]), "reason"),
        "the script ended with an open batch"
    );
    // Nothing partial was ever published: the draft holds only the fork revision, and no
    // plate exists.
    let (code, draft_head) = StoreFixture::run_in(&draft_arg, "open-revision", &[]);
    assert_eq!(code, 0);
    assert_eq!(text(&draft_head, "sequence"), "1");
    let (code, empty) = StoreFixture::run_in(&draft_arg, "inspect-elements", &[]);
    assert_eq!(code, 0);
    assert_eq!(list(&empty, "plates").len(), 0);
}

#[test]
fn a_broken_script_file_is_a_typed_refusal_that_touches_nothing() {
    let f = fixture("script-broken");
    let store_arg = f.root.join("store").to_str().unwrap().to_owned();
    f.run("define-material", &["--grade", "RS", "--density", "7850"]);
    // A missing file is a typed refusal with exit 2; the message names the script
    // problem, not a command refusal.
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args([
            "run-script",
            "--store",
            &store_arg,
            "--file",
            f.root.join("no-such-script.json").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let missing = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
    assert_eq!(text(&missing, "outcome"), "rejected");
    assert!(text(&missing, "message").contains("script"), "{missing:?}");
    let malformed = f.root.join("malformed.json");
    fs::write(&malformed, "[[\"create-plate\"").unwrap();
    let (code, broken) = raw(&store_arg, &malformed, &[]);
    assert_eq!(code, 2);
    assert!(text(&broken, "message").contains("JSON"), "{broken:?}");
    let not_array = f.root.join("not-array.json");
    fs::write(&not_array, r#"["create-plate"]"#).unwrap();
    let (code, shape) = raw(&store_arg, &not_array, &[]);
    assert_eq!(code, 2);
    assert!(
        text(&shape, "message").contains("array of option tokens"),
        "{shape:?}"
    );
    // The store stands: the head is still the definition revision.
    let (code, head) = f.run("open-revision", &[]);
    assert_eq!(code, 0);
    assert_eq!(text(&head, "sequence"), "1");
    // The options of run-script itself: an unknown one is refused, --file is required.
    let good = f.root.join("good.json");
    fs::write(&good, "[]").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args([
            "run-script",
            "--store",
            &store_arg,
            "--file",
            good.to_str().unwrap(),
            "--tier",
            "core",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let unknown = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
    assert!(text(&unknown, "message").contains("unknown"), "{unknown:?}");
    let empty_file = f.root.join("empty.json");
    fs::write(&empty_file, "[]").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args(["run-script", "--store", &store_arg, "--file"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let dangling = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
    assert!(
        text(&dangling, "message").contains("a value is missing"),
        "{dangling:?}"
    );
    // Without --file at all the refusal names the missing option.
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args(["run-script", "--store", &store_arg])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let needs_file = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
    assert!(
        text(&needs_file, "message").contains("--file is required"),
        "{needs_file:?}"
    );
}

#[test]
fn a_nested_script_and_a_session_are_refused_inside_a_script() {
    let f = fixture("script-nested");
    let store = f.root.join("store");
    let store_arg = store.to_str().unwrap().to_owned();
    let replies = f.script(
        &store,
        &[
            Scripted::refused(&[
                "run-script",
                "--store",
                &store_arg,
                "--file",
                "no-such.json",
            ]),
            Scripted::refused(&["session", "--store", &store_arg]),
        ],
    );
    for reply in &replies {
        assert_eq!(text(reply, "outcome"), "rejected");
        assert!(
            text(reply, "message").contains("cannot run inside a session"),
            "{reply:?}"
        );
    }
}
