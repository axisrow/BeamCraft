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
fn a_batched_attach_resolves_a_part_created_earlier_in_the_same_script() {
    // #767: a command inside a batch resolves its targets from the script's working state,
    // exactly as the interactive path does. The assembly commits alone, then the plate is
    // created inside the batch and the attach in a later command of the same script must
    // resolve it from the draft — refusing it against the committed head used to kill the
    // script mid-way with NOT_FOUND and abandon the batch.
    let f = fixture("script-batch-attach");
    let store_arg = f.root.join("store").to_str().unwrap().to_owned();
    let (code, summary) = raw(
        &store_arg,
        &f.root.join("batch-attach.json"),
        &[
            &[
                "create-assembly",
                "--store",
                &store_arg,
                "--name",
                "Frame",
                "--new-id",
                "019a7bf0-0000-7000-8000-000000400001",
            ],
            &["batch-open"],
            &[
                "create-plate",
                "--store",
                &store_arg,
                "--width",
                "200",
                "--length",
                "200",
                "--thickness",
                "10",
                "--new-id",
                "019a7bf0-0000-7000-8000-000000400002",
                "--name",
                "Wing",
            ],
            &[
                "attach-to-assembly",
                "--store",
                &store_arg,
                "--id",
                "019a7bf0-0000-7000-8000-000000400002",
                "--assembly-id",
                "019a7bf0-0000-7000-8000-000000400001",
            ],
            &["batch-commit"],
        ],
    );
    assert_eq!(code, 0);
    assert_eq!(
        at(&summary, &["failed"]).unsigned().unwrap(),
        0,
        "{summary:?}"
    );
    let replies = list(&summary, "replies");
    let outcomes: Vec<String> = replies
        .iter()
        .map(|r| text(at(r, &["reply"]), "outcome"))
        .collect();
    assert_eq!(
        outcomes,
        [
            "assembly_created",
            "batch_opened",
            "plate_created",
            "part_attached",
            "batch_committed"
        ]
    );
    // The two modelled commands landed as one revision; a separate one-shot process sees
    // the attachment in the committed store.
    let (code, head) = StoreFixture::run_in(&store_arg, "open-revision", &[]);
    assert_eq!(code, 0);
    assert_eq!(text(&head, "sequence"), "2");
    let (code, view) = StoreFixture::run_in(&store_arg, "assemblies", &[]);
    assert_eq!(code, 0);
    let assemblies = list(&view, "assemblies");
    assert_eq!(assemblies.len(), 1);
    assert_eq!(text(&assemblies[0], "name"), "Frame");
    assert_eq!(list(&assemblies[0], "parts").len(), 1);
}

#[test]
fn a_batched_update_resolves_the_plate_created_earlier_in_the_same_script() {
    // Review round of #767: `update-element` is a `Route::Plate` command and batchable like
    // the attach, so the same seam flip fixes it. The plate exists only in the draft when
    // the later command of the same script patches it; the committed store must end up with
    // the patched thickness, published as one revision.
    let f = fixture("script-batch-update");
    let store_arg = f.root.join("store").to_str().unwrap().to_owned();
    let id = "019a7bf0-0000-7000-8000-000000400003";
    let (code, summary) = raw(
        &store_arg,
        &f.root.join("batch-update.json"),
        &[
            // The first revision belongs outside a batch (`batch-open` refuses an empty store).
            &[
                "define-material",
                "--store",
                &store_arg,
                "--grade",
                "RS",
                "--density",
                "7850",
            ],
            &["batch-open"],
            &[
                "create-plate",
                "--store",
                &store_arg,
                "--width",
                "200",
                "--length",
                "200",
                "--thickness",
                "10",
                "--new-id",
                id,
                "--name",
                "Wing",
            ],
            &[
                "update-element",
                "--store",
                &store_arg,
                "--id",
                id,
                "--thickness",
                "12",
            ],
            &["batch-commit"],
        ],
    );
    assert_eq!(code, 0);
    assert_eq!(
        at(&summary, &["failed"]).unsigned().unwrap(),
        0,
        "{summary:?}"
    );
    let replies = list(&summary, "replies");
    let outcomes: Vec<String> = replies
        .iter()
        .map(|r| text(at(r, &["reply"]), "outcome"))
        .collect();
    assert_eq!(
        outcomes,
        [
            "definition_created",
            "batch_opened",
            "plate_created",
            "element_updated",
            "batch_committed"
        ]
    );
    assert_eq!(
        at(&replies[4], &["reply", "commands"]).unsigned().unwrap(),
        2
    );
    // A separate one-shot process sees the patched plate in the committed store.
    let (code, view) = StoreFixture::run_in(&store_arg, "inspect-elements", &[]);
    assert_eq!(code, 0);
    let plates = list(&view, "plates");
    assert_eq!(plates.len(), 1);
    assert_eq!(
        at(&plates[0], &["dimensions_mm", "thickness"])
            .float()
            .unwrap(),
        12.0
    );
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
fn a_refused_batch_word_counts_as_failed_in_the_script_summary() {
    // Review of the 1119 batch replay: the batch words hardwired exit 0, so a batch word
    // that refused summarized as failed 0 with process exit 0, against the promise that
    // the replies carry each command's exit code.
    let f = fixture("script-batch-refusals");
    let store_arg = f.root.join("store").to_str().unwrap().to_owned();
    f.run("define-material", &["--grade", "RS", "--density", "7850"]);
    let (code, summary) = raw(
        &store_arg,
        &f.root.join("refusals.json"),
        &[
            &["batch-commit"],
            &["batch-abort"],
            &["open-revision", "--store", &store_arg],
        ],
    );
    // The process still exits 0 — a refusal is a summary entry — but the refused words
    // carry their own exits and the failed count sees them.
    assert_eq!(code, 0);
    assert_eq!(text(&summary, "outcome"), "script_complete");
    assert_eq!(at(&summary, &["commands"]).unsigned().unwrap(), 3);
    assert_eq!(at(&summary, &["failed"]).unsigned().unwrap(), 2);
    let replies = list(&summary, "replies");
    assert_eq!(at(&replies[0], &["exit"]).unsigned().unwrap(), 2);
    assert_eq!(
        text(at(&replies[0], &["reply"]), "code"),
        "INVALID_ARGUMENT"
    );
    assert_eq!(at(&replies[1], &["exit"]).unsigned().unwrap(), 2);
    // The read after the refusals still answers: the session stays alive.
    assert_eq!(at(&replies[2], &["exit"]).unsigned().unwrap(), 0);
    assert_eq!(text(at(&replies[2], &["reply"]), "sequence"), "1");
}

#[test]
fn a_second_batch_open_refuses_and_abandons_the_open_batch() {
    // The refusal exit of the batch words rides the same abandonment rule as any refused
    // command: the draft is open after the refused line, so the batch abandons whole.
    let f = fixture("script-batch-double-open");
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
    let (code, summary) = raw(
        &draft_arg,
        &f.root.join("double-open.json"),
        &[&["batch-open"], &["batch-open"]],
    );
    assert_eq!(code, 0);
    assert_eq!(at(&summary, &["failed"]).unsigned().unwrap(), 1);
    let replies = list(&summary, "replies");
    // The refusal line, then the abandonment line after it.
    assert_eq!(replies.len(), 3);
    assert_eq!(at(&replies[1], &["exit"]).unsigned().unwrap(), 2);
    assert_eq!(
        text(at(&replies[2], &["reply"]), "outcome"),
        "batch_abandoned"
    );
    // The abandoned batch published nothing.
    let (code, draft_head) = StoreFixture::run_in(&draft_arg, "open-revision", &[]);
    assert_eq!(code, 0);
    assert_eq!(text(&draft_head, "sequence"), "1");
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
