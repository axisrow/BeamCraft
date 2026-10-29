// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates and workspaces, MPL-2.0.

//! C01 end to end through the CLI binary: a workspace's unsent revisions become one changeset
//! (base, operations, actor, dependencies, USD delta, assets); receiving it publishes the draft
//! head exactly once. The same payload replays, another payload under the same changeset id is
//! `OP_ID_REUSE` with nothing written, and the author string in the payload grants nothing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support;

use support::{at, text};

use beamcraft_host::{revision::RevisionStore, storage::BlobStore};
use beamcraft_json::{Json, Limits};
use std::fs;
use std::process::Command;

const USER: &str = "019a7bf0-0000-7000-8000-00000000c001";
const STRANGER: &str = "019a7bf0-0000-7000-8000-00000000c002";
const PLATE: &str = "019a7bf0-0000-7000-8000-000000000010";
const CHANGESET: &str = "019a7bf0-0000-7000-8000-00000000d001";
const REVISION: &str = "019a7bf0-0000-7000-8000-00000000d002";

struct Fixture {
    shared: support::StoreFixture,
}
impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("c01-tests"),
        };
        fs::create_dir(fixture.shared.root.join("published")).unwrap();
        fixture
    }
    fn path(&self, name: &str) -> String {
        self.shared.root.join(name).to_str().unwrap().to_owned()
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
    fn rejected(&self, args: &[&str]) -> String {
        let before = self.head();
        let (code, body) = self.run(args);
        assert_eq!(code, 2, "{args:?}: {body:?}");
        assert_eq!(self.head(), before, "a refusal publishes nothing: {args:?}");
        text(&body, "code")
    }
    fn head(&self) -> String {
        let store = self.path("published");
        text(
            &self.ok(&["open-revision", "--store", &store]),
            "revision_id",
        )
    }
    /// A published plate and a workspace of `USER` forked from it.
    fn forked(&self, workspace: &str) -> String {
        let store = self.path("published");
        if self.run(&["open-revision", "--store", &store]).0 != 0 {
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
        let dir = self.path(workspace);
        self.ok(&[
            "create-workspace",
            "--store",
            &store,
            "--workspace-dir",
            &dir,
            "--user-id",
            USER,
        ]);
        dir
    }
    fn rename(&self, dir: &str, name: &str) {
        self.ok(&[
            "update-element",
            "--store",
            dir,
            "--id",
            PLATE,
            "--name",
            name,
        ]);
    }
    fn create(&self, dir: &str, extra: &[&str]) -> Json {
        let store = self.path("published");
        let mut args = vec![
            "create-changeset",
            "--store",
            &store,
            "--workspace-dir",
            dir,
        ];
        args.extend_from_slice(extra);
        at(&self.ok(&args), &["changeset"]).clone()
    }
    fn apply_args<'a>(
        &'a self,
        store: &'a str,
        dir: &'a str,
        digest: &'a str,
        actor: &'a str,
    ) -> [&'a str; 9] {
        [
            "apply-changeset",
            "--store",
            store,
            "--workspace-dir",
            dir,
            "--changeset",
            digest,
            "--actor-id",
            actor,
        ]
    }
}

fn list<'a>(json: &'a Json, key: &str) -> &'a [Json] {
    at(json, &[key]).array().unwrap()
}
fn dependencies(changeset: &Json, target: &str) -> Vec<(String, String)> {
    list(changeset, "dependencies")
        .iter()
        .filter(|d| text(d, "target") == target)
        .map(|d| (text(d, "group"), text(d, "access")))
        .collect()
}

fn writes(changeset: &Json, target: &str) -> Vec<String> {
    dependencies(changeset, target)
        .into_iter()
        .filter(|(_, access)| access == "write")
        .map(|(group, _)| group)
        .collect()
}
fn reads(changeset: &Json, target: &str) -> Vec<String> {
    dependencies(changeset, target)
        .into_iter()
        .filter(|(_, access)| access == "read")
        .map(|(group, _)| group)
        .collect()
}

#[test]
fn a_changeset_carries_base_operations_dependencies_and_delta_and_publishes_once() {
    let f = Fixture::new();
    let dir = f.forked("ws");
    let base = f.head();
    f.rename(&dir, "Renamed");
    f.rename(&dir, "Renamed twice");
    let ids = ["--changeset-id", CHANGESET, "--revision-id", REVISION];
    let changeset = f.create(&dir, &ids);
    // Same identities on an unchanged draft: the same canonical bytes.
    assert_eq!(
        text(&f.create(&dir, &ids), "digest"),
        text(&changeset, "digest")
    );
    assert_eq!(text(&changeset, "base_revision"), base);
    assert_eq!(text(&changeset, "actor_id"), USER);
    assert_eq!(list(&changeset, "operations").len(), 2);
    // Captured commands preserve the exact label/lifecycle reads instead of all model groups.
    assert_eq!(writes(&changeset, PLATE), ["label"]);
    assert_eq!(reads(&changeset, PLATE), ["label", "lifecycle"]);
    let changed: Vec<String> = list(&changeset, "layers")
        .iter()
        .filter(|l| at(l, &["changed"]) == &Json::Bool(true))
        .map(|l| text(l, "name"))
        .collect();
    assert_eq!(changed, ["authored.usda"]);
    assert_eq!(list(&changeset, "assets").len(), 1);
    let digest = text(&changeset, "digest");
    let inspected = f.ok(&[
        "inspect-changeset",
        "--workspace-dir",
        &dir,
        "--changeset",
        &digest,
    ]);
    assert_eq!(at(&inspected, &["changeset"]), &changeset);

    let store = f.path("published");
    let applied = f.ok(&f.apply_args(&store, &dir, &digest, USER));
    assert_eq!(text(&applied, "outcome"), "changeset_applied");
    assert_eq!(at(&applied, &["workspace_synced"]), &Json::Bool(true));
    let head = f.ok(&["open-revision", "--store", &store]);
    assert_eq!(text(&head, "revision_id"), REVISION);
    assert_eq!(text(&head, "op_id"), CHANGESET);
    let names: Vec<String> = list(&f.ok(&["inspect-elements", "--store", &store]), "plates")
        .iter()
        .map(|p| text(p, "name"))
        .collect();
    assert_eq!(names, ["Renamed twice"]);
    // The workspace base moved: nothing is unsent.
    let status = f.ok(&[
        "inspect-workspace",
        "--store",
        &store,
        "--workspace-dir",
        &dir,
    ]);
    assert_eq!(text(at(&status, &["workspace"]), "unsent_revisions"), "0");

    // A retry of the same payload is a replay that writes nothing.
    let replay = f.ok(&f.apply_args(&store, &dir, &digest, USER));
    assert_eq!(text(&replay, "outcome"), "changeset_replayed");
    assert_eq!(text(at(&replay, &["revision"]), "revision_id"), REVISION);
    assert_eq!(f.head(), REVISION);
}

#[test]
fn the_author_string_grants_nothing_and_another_payload_under_the_id_is_reuse() {
    let f = Fixture::new();
    let dir = f.forked("ws");
    let store = f.path("published");
    f.rename(&dir, "One");
    let ids = ["--changeset-id", CHANGESET, "--revision-id", REVISION];
    let first = text(&f.create(&dir, &ids), "digest");
    // The caller is not the claimed actor, before and after the id is bound.
    assert_eq!(
        f.rejected(&f.apply_args(&store, &dir, &first, STRANGER)),
        "PERMISSION_DENIED"
    );
    // A payload that claims a stranger as its author is refused for the real caller too.
    let forged = text(
        &f.create(
            &dir,
            &[
                "--changeset-id",
                "019a7bf0-0000-7000-8000-00000000d0f0",
                "--actor-id",
                STRANGER,
            ],
        ),
        "digest",
    );
    assert_eq!(
        f.rejected(&f.apply_args(&store, &dir, &forged, USER)),
        "PERMISSION_DENIED"
    );
    f.ok(&f.apply_args(&store, &dir, &first, USER));
    assert_eq!(
        f.rejected(&f.apply_args(&store, &dir, &first, STRANGER)),
        "PERMISSION_DENIED"
    );
    // The same changeset id with other content: refused, nothing written.
    f.rename(&dir, "Two");
    let other = text(
        &f.create(
            &dir,
            &[
                "--changeset-id",
                CHANGESET,
                "--revision-id",
                "019a7bf0-0000-7000-8000-00000000d003",
            ],
        ),
        "digest",
    );
    assert_ne!(other, first);
    assert_eq!(
        f.rejected(&f.apply_args(&store, &dir, &other, USER)),
        "OP_ID_REUSE"
    );
}

#[test]
fn a_moved_head_is_refused_and_the_refused_id_stays_bound() {
    let f = Fixture::new();
    let one = f.forked("one");
    let two = f.forked("two");
    let store = f.path("published");
    f.rename(&one, "From one");
    f.rename(&two, "From two");
    let first = text(&f.create(&one, &[]), "digest");
    f.ok(&f.apply_args(&store, &one, &first, USER));
    let ids = ["--changeset-id", CHANGESET];
    let second = text(&f.create(&two, &ids), "digest");
    assert_eq!(
        f.rejected(&f.apply_args(&store, &two, &second, USER)),
        "BASE_REVISION_MISMATCH"
    );
    // A retry of the same payload is refused the same way, never as reuse.
    assert_eq!(
        f.rejected(&f.apply_args(&store, &two, &second, USER)),
        "BASE_REVISION_MISMATCH"
    );
    // The id names that payload forever: new content needs a new changeset id.
    f.rename(&two, "From two again");
    let changed = text(&f.create(&two, &ids), "digest");
    assert_eq!(
        f.rejected(&f.apply_args(&store, &two, &changed, USER)),
        "OP_ID_REUSE"
    );
    // The unsent draft is untouched.
    let status = f.ok(&[
        "inspect-workspace",
        "--store",
        &store,
        "--workspace-dir",
        &two,
    ]);
    assert_eq!(text(at(&status, &["workspace"]), "unsent_revisions"), "2");
}

#[test]
fn bounded_validation_refuses_garbage_missing_and_empty_changesets() {
    let f = Fixture::new();
    let dir = f.forked("ws");
    let store = f.path("published");
    // Nothing to send.
    let (code, body) = f.run(&[
        "create-changeset",
        "--store",
        &store,
        "--workspace-dir",
        &dir,
    ]);
    assert_eq!(
        (code, text(&body, "code")),
        (2, "INVALID_ARGUMENT".to_owned())
    );
    // A blob that is not a changeset.
    let workspace = RevisionStore::new(BlobStore::new(&dir).unwrap());
    let (garbage, _) = workspace.blobs().publish(b"not a changeset").unwrap();
    assert_eq!(
        f.rejected(&f.apply_args(&store, &dir, &garbage.to_string(), USER)),
        "MALFORMED_COMMAND"
    );
    let (future, _) = workspace
        .blobs()
        .publish(b"BeamCraft/C01/Changeset/v9\0 and a body long enough")
        .unwrap();
    assert_eq!(
        f.rejected(&f.apply_args(&store, &dir, &future.to_string(), USER)),
        "UNSUPPORTED_VERSION"
    );
    // A real changeset with one byte appended is not canonical.
    f.rename(&dir, "Renamed");
    let digest = text(&f.create(&dir, &[]), "digest");
    let mut bytes = workspace.blobs().read(&digest.parse().unwrap()).unwrap();
    bytes.push(0);
    let (longer, _) = workspace.blobs().publish(&bytes).unwrap();
    assert_eq!(
        f.rejected(&f.apply_args(&store, &dir, &longer.to_string(), USER)),
        "MALFORMED_COMMAND"
    );
    // An unknown digest.
    assert_eq!(
        f.rejected(&f.apply_args(&store, &dir, &"ab".repeat(32), USER)),
        "NOT_FOUND"
    );
}

#[test]
fn a_copy_reads_its_unchanged_source_and_a_move_writes_the_derived_invalidation() {
    let f = Fixture::new();
    let dir = f.forked("ws");
    let copy = "019a7bf0-0000-7000-8000-000000000020";
    f.ok(&[
        "copy-element",
        "--store",
        &dir,
        "--id",
        PLATE,
        "--new-id",
        copy,
        "--offset",
        "2000,0,0",
    ]);
    let changeset = f.create(&dir, &[]);
    // The source is unchanged, yet the copy read all of it (Footprint::for_copy).
    assert!(writes(&changeset, PLATE).is_empty());
    assert_eq!(
        reads(&changeset, PLATE),
        [
            "label",
            "shape_inputs",
            "placement",
            "material_ref",
            "relationship",
            "lifecycle"
        ]
    );
    assert_eq!(writes(&changeset, copy).len(), 7);
    let dir = f.forked("moved");
    f.ok(&[
        "update-element",
        "--store",
        &dir,
        "--id",
        PLATE,
        "--translation",
        "10,20,30",
    ]);
    let changeset = f.create(&dir, &[]);
    assert_eq!(writes(&changeset, PLATE), ["placement", "derived"]);
}

#[test]
fn a_changeset_id_that_names_another_committed_operation_is_reuse_every_time() {
    let f = Fixture::new();
    let store = f.path("published");
    let create_op = "019a7bf0-0000-7000-8000-00000000e001";
    f.ok(&[
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
        "--op-id",
        create_op,
    ]);
    let dir = f.forked("ws");
    f.rename(&dir, "Renamed");
    let digest = text(&f.create(&dir, &["--changeset-id", create_op]), "digest");
    for _ in 0..2 {
        assert_eq!(
            f.rejected(&f.apply_args(&store, &dir, &digest, USER)),
            "OP_ID_REUSE"
        );
    }
    let status = f.ok(&[
        "inspect-workspace",
        "--store",
        &store,
        "--workspace-dir",
        &dir,
    ]);
    assert_eq!(text(at(&status, &["workspace"]), "unsent_revisions"), "1");
}

#[test]
fn an_assembly_membership_writes_the_part_relationship_and_publishes() {
    let f = Fixture::new();
    let dir = f.forked("ws");
    let assembly = "019a7bf0-0000-7000-8000-000000000030";
    f.ok(&[
        "create-assembly",
        "--store",
        &dir,
        "--name",
        "Frame",
        "--new-id",
        assembly,
    ]);
    f.ok(&[
        "attach-to-assembly",
        "--store",
        &dir,
        "--id",
        PLATE,
        "--assembly-id",
        assembly,
    ]);
    let changeset = f.create(&dir, &[]);
    // The part's one bim:partOf record is its relationship group (Footprint::for_part_of); the
    // new assembly writes every group.
    assert_eq!(writes(&changeset, PLATE), ["relationship"]);
    assert_eq!(writes(&changeset, assembly).len(), 7);
    let store = f.path("published");
    let digest = text(&changeset, "digest");
    let applied = f.ok(&f.apply_args(&store, &dir, &digest, USER));
    assert_eq!(text(&applied, "outcome"), "changeset_applied");
    let all = f.ok(&["assemblies", "--store", &store]);
    let published = list(&all, "assemblies");
    assert_eq!(published.len(), 1);
    assert_eq!(text(&published[0], "id"), assembly);
    let parts: Vec<String> = list(&published[0], "parts")
        .iter()
        .map(|p| p.string().unwrap().to_owned())
        .collect();
    assert_eq!(parts, [PLATE]);
}
