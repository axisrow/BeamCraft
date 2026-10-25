// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, MPL-2.0. Expected values are closed forms.

//! P06 end to end: a user workspace is a separate draft store forked from the published head.
//! Edits land in the draft only; the published head moves only on a fast-forward `publish-workspace`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_json::{Json, Limits};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("p06-tests");
        fs::create_dir_all(&base).unwrap();
        let root = base.join(format!(
            "e2e-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("published")).unwrap();
        Self { root }
    }
    fn dir(&self, name: &str) -> String {
        self.root.join(name).to_str().unwrap().to_owned()
    }
    fn run_in(&self, store: &str, command: &str, extra: &[&str]) -> (i32, Json) {
        let mut args = vec![command, "--store", store];
        args.extend_from_slice(extra);
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(&args)
            .output()
            .unwrap();
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
        (output.status.code().unwrap(), body)
    }
    fn ok_in(&self, store: &str, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run_in(store, command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    }
    fn code_in(&self, store: &str, command: &str, extra: &[&str]) -> String {
        let (code, body) = self.run_in(store, command, extra);
        assert_eq!(code, 2, "{command} {extra:?}: {body:?}");
        text(&body, "code")
    }
    fn published(&self) -> String {
        self.dir("published")
    }
    fn plate(&self, store: &str, name: &str) -> String {
        let created = self.ok_in(
            store,
            "create-plate",
            &[
                "--width",
                "1000",
                "--length",
                "500",
                "--thickness",
                "10",
                "--name",
                name,
            ],
        );
        text(at(&created, &["plate"]), "element_id")
    }
    fn names(&self, store: &str) -> Vec<String> {
        let mut names: Vec<String> = at(&self.ok_in(store, "inspect-elements", &[]), &["plates"])
            .array()
            .unwrap()
            .iter()
            .map(|p| text(p, "name"))
            .collect();
        names.sort();
        names
    }
    fn pin(&self, store: &str) -> String {
        let inspected = self.ok_in(store, "inspect-elements", &[]);
        text(at(&inspected, &["snapshot"]), "pin_revision")
    }
    fn fork(&self, name: &str, user: &str) -> (String, Json) {
        let dir = self.dir(name);
        let created = self.ok_in(
            &self.published(),
            "create-workspace",
            &["--workspace-dir", &dir, "--user-id", user],
        );
        (dir, created)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.root.is_absolute() && self.root.file_name().is_some());
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn text(json: &Json, key: &str) -> String {
    json.get(key).unwrap().string().unwrap().to_owned()
}
fn at<'a>(json: &'a Json, path: &[&str]) -> &'a Json {
    path.iter().fold(json, |j, k| j.get(k).unwrap())
}
fn head(status: &Json, which: &str) -> String {
    text(at(status, &["workspace", which]), "manifest_digest")
}

const USER_A: &str = "01890a5d-ac96-774b-b9cc-aaaaaaaaaaaa";
const USER_B: &str = "01890a5d-ac96-774b-b9cc-bbbbbbbbbbbb";

#[test]
fn a_workspace_edits_a_draft_and_publishes_by_fast_forward() {
    let f = Fixture::new();
    let published = f.published();
    let id = f.plate(&published, "Base plate");
    let published_before = f.pin(&published);

    let (dir, created) = f.fork("ws-a", USER_A);
    assert_eq!(text(&created, "outcome"), "workspace_created");
    assert_eq!(text(at(&created, &["workspace"]), "unsent_revisions"), "0");
    assert_eq!(head(&created, "published_head"), published_before);
    assert_eq!(f.names(&dir), ["Base plate"]);

    // Edits land in the draft only.
    f.ok_in(
        &dir,
        "update-element",
        &["--id", &id, "--name", "Draft name"],
    );
    f.ok_in(
        &dir,
        "copy-element",
        &["--id", &id, "--name", "Copy", "--offset", "2000,0,0"],
    );
    assert_eq!(f.names(&dir), ["Copy", "Draft name"]);
    assert_eq!(f.names(&published), ["Base plate"]);
    assert_eq!(f.pin(&published), published_before);

    let status = f.ok_in(&published, "inspect-workspace", &["--workspace-dir", &dir]);
    assert_eq!(text(at(&status, &["workspace"]), "unsent_revisions"), "2");
    assert_eq!(
        at(&status, &["workspace", "base_is_current"]),
        &Json::Bool(true)
    );

    let sent = f.ok_in(&published, "publish-workspace", &["--workspace-dir", &dir]);
    assert_eq!(text(&sent, "outcome"), "workspace_published");
    assert_eq!(f.names(&published), ["Copy", "Draft name"]);
    assert_ne!(f.pin(&published), published_before);
    let status = f.ok_in(&published, "inspect-workspace", &["--workspace-dir", &dir]);
    assert_eq!(text(at(&status, &["workspace"]), "unsent_revisions"), "0");
    assert_eq!(head(&status, "published_head"), f.pin(&published));

    // Publishing again is an idempotent replay and moves nothing.
    let pinned = f.pin(&published);
    let replay = f.ok_in(&published, "publish-workspace", &["--workspace-dir", &dir]);
    assert_eq!(text(&replay, "outcome"), "workspace_publish_replayed");
    assert_eq!(f.pin(&published), pinned);

    // A fresh fork has nothing to publish.
    let (fresh, _) = f.fork("ws-fresh", USER_A);
    assert_eq!(
        f.code_in(
            &published,
            "publish-workspace",
            &["--workspace-dir", &fresh]
        ),
        "INVALID_ARGUMENT"
    );
}

#[test]
fn two_workspaces_of_one_user_are_independent_and_the_second_publisher_conflicts() {
    let f = Fixture::new();
    let published = f.published();
    let id = f.plate(&published, "Base plate");
    let (one, _) = f.fork("ws-one", USER_A);
    let (two, _) = f.fork("ws-two", USER_A);
    f.ok_in(&one, "update-element", &["--id", &id, "--name", "From one"]);
    f.ok_in(&two, "update-element", &["--id", &id, "--name", "From two"]);
    assert_eq!(f.names(&one), ["From one"]);
    assert_eq!(f.names(&two), ["From two"]);

    f.ok_in(&published, "publish-workspace", &["--workspace-dir", &one]);
    assert_eq!(f.names(&published), ["From one"]);
    let pinned = f.pin(&published);

    // The published head moved, so the other draft cannot fast-forward; it is left untouched.
    assert_eq!(
        f.code_in(&published, "publish-workspace", &["--workspace-dir", &two]),
        "BASE_REVISION_MISMATCH"
    );
    assert_eq!(f.pin(&published), pinned);
    assert_eq!(f.names(&two), ["From two"]);
    let status = f.ok_in(&published, "inspect-workspace", &["--workspace-dir", &two]);
    assert_eq!(
        at(&status, &["workspace", "base_is_current"]),
        &Json::Bool(false)
    );
    assert_eq!(text(at(&status, &["workspace"]), "unsent_revisions"), "1");
}

#[test]
fn unsent_work_survives_a_restart_and_a_repeated_create_resumes() {
    let f = Fixture::new();
    let published = f.published();
    let id = f.plate(&published, "Base plate");
    let (dir, created) = f.fork("ws-a", USER_A);
    let workspace_id = text(at(&created, &["workspace"]), "workspace_id");
    f.ok_in(&dir, "update-element", &["--id", &id, "--name", "Unsent"]);

    // A fresh process sees the same draft; repeating the create with the same identity resumes.
    let again = f.ok_in(
        &published,
        "create-workspace",
        &[
            "--workspace-dir",
            &dir,
            "--user-id",
            USER_A,
            "--workspace-id",
            &workspace_id,
        ],
    );
    assert_eq!(text(at(&again, &["workspace"]), "unsent_revisions"), "1");
    assert_eq!(f.names(&dir), ["Unsent"]);

    // Another user's workspace cannot take over the directory.
    assert_eq!(
        f.code_in(
            &published,
            "create-workspace",
            &["--workspace-dir", &dir, "--user-id", USER_B]
        ),
        "DUPLICATE_ID"
    );
}

#[test]
fn bad_workspaces_are_rejected_without_writing() {
    let f = Fixture::new();
    let published = f.published();
    let dir = f.dir("ws-a");
    // Nothing is published yet.
    assert_eq!(
        f.code_in(
            &published,
            "create-workspace",
            &["--workspace-dir", &dir, "--user-id", USER_A]
        ),
        "NOT_FOUND"
    );
    f.plate(&published, "Base plate");
    assert_eq!(
        f.code_in(&published, "inspect-workspace", &[]),
        "INVALID_ARGUMENT"
    );
    // The published store is never a workspace.
    assert_eq!(
        f.code_in(
            &published,
            "create-workspace",
            &["--workspace-dir", &published, "--user-id", USER_A]
        ),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.code_in(
            &published,
            "create-workspace",
            &["--workspace-dir", &dir, "--user-id", "not-a-uuid"]
        ),
        "INVALID_ID"
    );
}

#[test]
fn publish_takes_its_id_from_the_draft_and_aliases_of_the_published_store_are_refused() {
    let f = Fixture::new();
    let published = f.published();
    let id = f.plate(&published, "Base plate");
    let (dir, _) = f.fork("ws-a", USER_A);
    f.ok_in(&dir, "update-element", &["--id", &id, "--name", "Draft"]);

    // The draft head's id is the published revision id, so a retry in any process is the same
    // request; a caller-chosen id would not be.
    assert_eq!(
        f.code_in(
            &published,
            "publish-workspace",
            &["--workspace-dir", &dir, "--revision-id", USER_B]
        ),
        "INVALID_ARGUMENT"
    );
    let draft_head = text(
        at(
            &f.ok_in(&published, "inspect-workspace", &["--workspace-dir", &dir]),
            &["workspace", "draft_head"],
        ),
        "revision_id",
    );
    f.ok_in(&published, "publish-workspace", &["--workspace-dir", &dir]);
    let inspected = f.ok_in(&published, "inspect-elements", &[]);
    assert_eq!(
        text(at(&inspected, &["revision"]), "revision_id"),
        draft_head
    );

    // `published/.` is the published store itself.
    let alias = format!("{published}{}.", std::path::MAIN_SEPARATOR);
    assert_eq!(
        f.code_in(
            &published,
            "create-workspace",
            &["--workspace-dir", &alias, "--user-id", USER_A]
        ),
        "INVALID_ARGUMENT"
    );
}
