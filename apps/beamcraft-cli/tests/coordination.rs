// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic coordination projects, MPL-2.0.

//! F12 core end to end through the real CLI binary (fork issue #364, U20 + U21):
//! the project descriptor with its section/zone registries of the 0.2 record shapes,
//! several independent section projects (KM and KMD, both `steel`) coordinated by a third
//! store through exact pinned revisions, the explicit verified update, and the retained
//! dependency closure across source publication, connection removal and compaction.
//!
//! Every rule gets its positive and negative executable case: rename keeps the identity,
//! repeated codes are `DUPLICATE_ID`, `latest` is never a pin, a stale expected pin is
//! `DEPENDENCY_CONFLICT`, an unknown alignment is `DEPENDENCY_UNAVAILABLE`, a non-rigid
//! placement is `UNSUPPORTED_GEOMETRY`, tampered pinned bytes are `INTEGRITY_ERROR`, and a
//! coordination revision keeps resolving after the source store disappears and a compaction.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{StoreFixture, at, list, text};

use beamcraft_json::Json;
use std::fs;

/// The boolean value at `key`, failing the test when absent or not a boolean.
fn flag(json: &Json, key: &str) -> bool {
    at(json, &[key]).boolean().unwrap()
}

const KM_PROJECT: &str = "019a7bf0-0000-7000-8000-00000000d001";
const KMD_PROJECT: &str = "019a7bf0-0000-7000-8000-00000000d002";
const COORD_PROJECT: &str = "019a7bf0-0000-7000-8000-00000000d003";
const KM_WORKSPACE: &str = "019a7bf0-0000-7000-8000-00000000d101";
const KMD_WORKSPACE: &str = "019a7bf0-0000-7000-8000-00000000d102";
const COORD_WORKSPACE: &str = "019a7bf0-0000-7000-8000-00000000d103";
const KM_SECTION: &str = "019a7bf0-0000-7000-8000-00000000d201";
const KMD_SECTION: &str = "019a7bf0-0000-7000-8000-00000000d202";
const ZONE_K1: &str = "019a7bf0-0000-7000-8000-00000000d301";
const CONNECTION_KM: &str = "019a7bf0-0000-7000-8000-00000000d401";
const CONNECTION_KMD: &str = "019a7bf0-0000-7000-8000-00000000d402";
const CONNECTION_NO_FRAME: &str = "019a7bf0-0000-7000-8000-00000000d403";

struct Fixture {
    shared: StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: StoreFixture::new("coordination-tests"),
        };
        for store in ["km", "kmd", "coord"] {
            fs::create_dir(fixture.shared.root.join(store)).unwrap();
        }
        fixture
    }

    /// Runs one command against one of the fixture's stores by name.
    fn run(&self, store: &str, command: &str, extra: &[&str]) -> (i32, Json) {
        StoreFixture::run_in(
            self.shared.root.join(store).to_str().unwrap(),
            command,
            extra,
        )
    }

    fn ok(&self, store: &str, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run(store, command, extra);
        assert_eq!(code, 0, "{command} on {store} {extra:?}: {body:?}");
        body
    }

    /// Asserts a refusal (exit 2) that writes no revision on that store.
    fn rejected(&self, store: &str, command: &str, extra: &[&str]) -> String {
        let before = self.head(store);
        let (code, body) = self.run(store, command, extra);
        assert_eq!(code, 2, "{command} on {store} {extra:?}: {body:?}");
        assert_eq!(self.head(store), before, "{command} wrote a revision");
        text(&body, "code")
    }

    fn head(&self, store: &str) -> String {
        let (code, body) = self.run(store, "open-revision", &[]);
        if code == 0 {
            text(&body, "revision_id")
        } else {
            String::new()
        }
    }

    fn with_ids<'a>(project: &'a str, workspace: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
        let mut args: Vec<&'a str> = vec!["--project-id", project, "--workspace-id", workspace];
        args.extend_from_slice(extra);
        args
    }

    fn steel_section(&self, store: &str, id: &str, code: &str, project: &str, workspace: &str) {
        self.ok(
            store,
            "define-section",
            &Self::with_ids(
                project,
                workspace,
                &[
                    "--code",
                    code,
                    "--module",
                    "steel",
                    "--section-id",
                    id,
                    "--label",
                    code,
                ],
            ),
        );
    }

    fn plate(&self, store: &str, project: &str, workspace: &str) -> String {
        let created = self.ok(
            store,
            "create-plate",
            &Self::with_ids(
                project,
                workspace,
                &["--width", "1000", "--length", "500", "--thickness", "10"],
            ),
        );
        text(at(&created, &["plate"]), "element_id")
    }

    /// Prepares one source store: the `steel` section, one plate moved to `x`, and returns
    /// its head revision.
    fn source_store(
        &self,
        store: &str,
        project: &str,
        workspace: &str,
        section: &str,
        code: &str,
    ) -> String {
        self.steel_section(store, section, code, project, workspace);
        let plate = self.plate(store, project, workspace);
        self.ok(
            store,
            "update-element",
            &Self::with_ids(
                project,
                workspace,
                &["--id", &plate, "--translation", "500,250,0"],
            ),
        );
        self.head(store)
    }
}

#[test]
fn descriptor_and_registries_survive_revisions_and_a_fork() {
    let f = Fixture::new();
    f.ok(
        "km",
        "set-project-metadata",
        &Fixture::with_ids(
            KM_PROJECT,
            KM_WORKSPACE,
            &[
                "--code",
                "2401-DEMO",
                "--name",
                "Demo warehouse",
                "--organization",
                "ACME",
            ],
        ),
    );
    f.steel_section("km", KM_SECTION, "KM", KM_PROJECT, KM_WORKSPACE);
    f.ok(
        "km",
        "define-zone",
        &Fixture::with_ids(
            KM_PROJECT,
            KM_WORKSPACE,
            &[
                "--code",
                "K1",
                "--zone-id",
                ZONE_K1,
                "--label",
                "Building K1",
            ],
        ),
    );

    // A repeated registry code is refused for another identity (PC-ID-001).
    assert_eq!(
        f.rejected(
            "km",
            "define-section",
            &Fixture::with_ids(
                KM_PROJECT,
                KM_WORKSPACE,
                &["--code", "KM", "--module", "steel", "--label", "again"],
            ),
        ),
        "DUPLICATE_ID"
    );
    assert_eq!(
        f.rejected(
            "km",
            "define-zone",
            &Fixture::with_ids(KM_PROJECT, KM_WORKSPACE, &["--code", "K1"]),
        ),
        "DUPLICATE_ID"
    );

    // Renaming keeps the identity: the same zone id, a new label.
    f.ok(
        "km",
        "define-zone",
        &Fixture::with_ids(
            KM_PROJECT,
            KM_WORKSPACE,
            &[
                "--code",
                "K1",
                "--zone-id",
                ZONE_K1,
                "--label",
                "Renamed K1",
            ],
        ),
    );

    // Ordinary modelling is not a shared-metadata command, but it carries the descriptor:
    // the metadata and both registries survive the revision unchanged (PC-OWN-002).
    f.plate("km", KM_PROJECT, KM_WORKSPACE);
    let shown = f.ok("km", "show-project", &["--project-id", KM_PROJECT]);
    let descriptor = at(&shown, &["descriptor"]);
    assert_eq!(text(descriptor, "code"), "2401-DEMO");
    assert_eq!(text(descriptor, "name"), "Demo warehouse");
    assert_eq!(text(descriptor, "organization"), "ACME");
    assert_eq!(text(descriptor, "frame"), "mm-rad-right-z-up");
    assert_eq!(text(at(descriptor, &["contract"]), "vocabulary"), "0.2.0");
    let sections = list(descriptor, "sections");
    assert_eq!(sections.len(), 1);
    assert_eq!(text(&sections[0], "id"), KM_SECTION);
    assert_eq!(text(&sections[0], "code"), "KM");
    let modules = list(&sections[0], "modules");
    assert_eq!(modules.len(), 1);
    assert_eq!(modules[0].string().unwrap(), "steel");
    let zones = list(descriptor, "zones");
    assert_eq!(zones.len(), 1);
    assert_eq!(text(&zones[0], "id"), ZONE_K1);
    assert_eq!(text(&zones[0], "label"), "Renamed K1");

    // The fork of the store carries the descriptor too (U20: metadata survive every fork).
    let fork = f.shared.root.join("km-fork");
    f.ok(
        "km",
        "create-workspace",
        &["--workspace-dir", fork.to_str().unwrap()],
    );
    let forked = f.ok(
        fork.to_str().unwrap(),
        "show-project",
        &["--project-id", KM_PROJECT],
    );
    assert_eq!(text(at(&forked, &["descriptor"]), "code"), "2401-DEMO");
    assert_eq!(list(at(&forked, &["descriptor"]), "sections").len(), 1);
}

#[test]
fn pins_are_exact_explicit_and_retained() {
    let f = Fixture::new();
    let km_revision = f.source_store("km", KM_PROJECT, KM_WORKSPACE, KM_SECTION, "KM");
    let kmd_revision = f.source_store("kmd", KMD_PROJECT, KMD_WORKSPACE, KMD_SECTION, "KMD");

    // The coordination store connects both independent section projects (the first proof:
    // two synthetic projects, the same steel module for KM and KMD).
    f.ok(
        "coord",
        "connect-project",
        &Fixture::with_ids(
            COORD_PROJECT,
            COORD_WORKSPACE,
            &[
                "--source-store",
                f.shared.root.join("km").to_str().unwrap(),
                "--source-project",
                KM_PROJECT,
                "--source-revision",
                &km_revision,
                "--connection-id",
                CONNECTION_KM,
                "--label",
                "KM steel structures",
                "--translation",
                "10000,0,0",
                "--orientation",
                "0,0,0,1",
            ],
        ),
    );
    f.ok(
        "coord",
        "connect-project",
        &Fixture::with_ids(
            COORD_PROJECT,
            COORD_WORKSPACE,
            &[
                "--source-store",
                f.shared.root.join("kmd").to_str().unwrap(),
                "--source-project",
                KMD_PROJECT,
                "--source-revision",
                &kmd_revision,
                "--connection-id",
                CONNECTION_KMD,
                "--label",
                "KMD detailing",
                "--scope",
                "KMD",
            ],
        ),
    );

    // The pins resolve completely against the head manifest.
    let shown = f.ok("coord", "show-project", &["--project-id", COORD_PROJECT]);
    let connections = list(at(&shown, &["descriptor"]), "connections");
    assert_eq!(connections.len(), 2);
    assert_eq!(text(&connections[0], "id"), CONNECTION_KM);
    assert_eq!(text(&connections[0], "source_revision"), km_revision);
    assert_eq!(list(&connections[0], "layers").len(), 3);
    assert_eq!(text(&connections[1], "scope"), "section");
    assert_eq!(text(&connections[1], "scope_id"), KMD_SECTION);
    let resolved = list(&shown, "connections");
    for status in resolved {
        assert!(flag(status, "complete"), "{status:?}");
        assert_eq!(list(status, "pins").len(), 3);
    }

    // PC-COORD-001: publishing a newer source revision leaves the pin unchanged.
    f.plate("km", KM_PROJECT, KM_WORKSPACE);
    let km_new_head = f.head("km");
    assert_ne!(km_new_head, km_revision);
    let shown = f.ok("coord", "show-project", &["--project-id", COORD_PROJECT]);
    assert_eq!(
        text(
            &list(at(&shown, &["descriptor"]), "connections")[0],
            "source_revision"
        ),
        km_revision
    );

    // There is no `latest` pin: only an exact revision id parses (PC-COORD-001 negative).
    assert_eq!(
        f.rejected(
            "coord",
            "connect-project",
            &Fixture::with_ids(
                COORD_PROJECT,
                COORD_WORKSPACE,
                &[
                    "--source-store",
                    f.shared.root.join("km").to_str().unwrap(),
                    "--source-project",
                    KM_PROJECT,
                    "--source-revision",
                    "latest",
                    "--connection-id",
                    CONNECTION_NO_FRAME,
                    "--label",
                    "not a pin",
                ],
            ),
        ),
        "INVALID_ID"
    );

    // PC-COORD-002: the explicit verified update to the new source revision, with a fixed
    // operation identity so the exact retry replays.
    let km_dir = f.shared.root.join("km").to_str().unwrap().to_owned();
    let update_args = Fixture::with_ids(
        COORD_PROJECT,
        COORD_WORKSPACE,
        &[
            "--connection",
            CONNECTION_KM,
            "--expected-revision",
            &km_revision,
            "--source-store",
            &km_dir,
            "--source-project",
            KM_PROJECT,
            "--source-revision",
            &km_new_head,
            "--op-id",
            "019a7bf0-0000-7000-8000-00000000e001",
            "--revision-id",
            "019a7bf0-0000-7000-8000-00000000e002",
        ],
    );
    let updated = f.ok("coord", "update-connection", &update_args);
    assert_eq!(text(&updated, "outcome"), "connection_updated");
    let shown = f.ok("coord", "show-project", &["--project-id", COORD_PROJECT]);
    assert_eq!(
        text(
            &list(at(&shown, &["descriptor"]), "connections")[0],
            "source_revision"
        ),
        km_new_head
    );

    // The exact retry of the same operation replays idempotently.
    let replay = f.ok("coord", "update-connection", &update_args);
    assert_eq!(text(&replay, "outcome"), "descriptor_replayed");

    // A stale expected pin is refused (PC-COORD-002 negative).
    assert_eq!(
        f.rejected(
            "coord",
            "update-connection",
            &Fixture::with_ids(
                COORD_PROJECT,
                COORD_WORKSPACE,
                &[
                    "--connection",
                    CONNECTION_KM,
                    "--expected-revision",
                    &km_revision,
                    "--source-store",
                    f.shared.root.join("km").to_str().unwrap(),
                    "--source-project",
                    KM_PROJECT,
                    "--source-revision",
                    &km_new_head,
                ],
            ),
        ),
        "DEPENDENCY_CONFLICT"
    );

    // The section-scoped pin survives an update without --scope: the saved selection is
    // preserved by identity and re-verified against the candidate's registry
    // (PC-COORD-003); it never widens to the whole project.
    f.plate("kmd", KMD_PROJECT, KMD_WORKSPACE);
    let kmd_new_head = f.head("kmd");
    f.ok(
        "coord",
        "update-connection",
        &Fixture::with_ids(
            COORD_PROJECT,
            COORD_WORKSPACE,
            &[
                "--connection",
                CONNECTION_KMD,
                "--expected-revision",
                &kmd_revision,
                "--source-store",
                f.shared.root.join("kmd").to_str().unwrap(),
                "--source-project",
                KMD_PROJECT,
                "--source-revision",
                &kmd_new_head,
            ],
        ),
    );
    let shown = f.ok("coord", "show-project", &["--project-id", COORD_PROJECT]);
    let kmd_connection = &list(at(&shown, &["descriptor"]), "connections")[1];
    assert_eq!(text(kmd_connection, "source_revision"), kmd_new_head);
    assert_eq!(text(kmd_connection, "scope"), "section");
    assert_eq!(text(kmd_connection, "scope_id"), KMD_SECTION);

    // A scope that resolves to no section or zone of the source registry is refused
    // (PC-ID-001 negative).
    assert_eq!(
        f.rejected(
            "coord",
            "connect-project",
            &Fixture::with_ids(
                COORD_PROJECT,
                COORD_WORKSPACE,
                &[
                    "--source-store",
                    f.shared.root.join("km").to_str().unwrap(),
                    "--source-project",
                    KM_PROJECT,
                    "--source-revision",
                    &km_new_head,
                    "--connection-id",
                    CONNECTION_NO_FRAME,
                    "--label",
                    "unknown scope",
                    "--scope",
                    "XX",
                ],
            ),
        ),
        "INVALID_ARGUMENT"
    );

    // A non-rigid placement is refused before anything is written (PC-FRAME-001 negative).
    assert_eq!(
        f.rejected(
            "coord",
            "connect-project",
            &Fixture::with_ids(
                COORD_PROJECT,
                COORD_WORKSPACE,
                &[
                    "--source-store",
                    f.shared.root.join("km").to_str().unwrap(),
                    "--source-project",
                    KM_PROJECT,
                    "--source-revision",
                    &km_new_head,
                    "--connection-id",
                    CONNECTION_NO_FRAME,
                    "--label",
                    "scaled",
                    "--translation",
                    "0,0,0",
                    "--orientation",
                    "0,0,0,2",
                ],
            ),
        ),
        "UNSUPPORTED_GEOMETRY"
    );

    // An unknown alignment resolves as an explicit unavailable state, never as identity.
    f.ok(
        "coord",
        "connect-project",
        &Fixture::with_ids(
            COORD_PROJECT,
            COORD_WORKSPACE,
            &[
                "--source-store",
                f.shared.root.join("km").to_str().unwrap(),
                "--source-project",
                KM_PROJECT,
                "--source-revision",
                &km_new_head,
                "--connection-id",
                CONNECTION_NO_FRAME,
                "--label",
                "no declared placement",
            ],
        ),
    );
    let (code, body) = f.run(
        "coord",
        "resolve-connection",
        &[
            "--project-id",
            COORD_PROJECT,
            "--connection",
            CONNECTION_NO_FRAME,
        ],
    );
    assert_eq!(code, 2, "{body:?}");
    assert_eq!(text(&body, "code"), "DEPENDENCY_UNAVAILABLE");
}

#[test]
fn known_transforms_give_independently_expected_positions() {
    let f = Fixture::new();
    let km_revision = f.source_store("km", KM_PROJECT, KM_WORKSPACE, KM_SECTION, "KM");
    f.ok(
        "coord",
        "connect-project",
        &Fixture::with_ids(
            COORD_PROJECT,
            COORD_WORKSPACE,
            &[
                "--source-store",
                f.shared.root.join("km").to_str().unwrap(),
                "--source-project",
                KM_PROJECT,
                "--source-revision",
                &km_revision,
                "--connection-id",
                CONNECTION_KM,
                "--label",
                "KM steel structures",
                // A quarter turn about Z followed by a translation along +Y.
                "--translation",
                "0,10000,0",
                "--orientation",
                "0,0,0.7071067811865476,0.7071067811865476",
            ],
        ),
    );
    let resolved = f.ok(
        "coord",
        "resolve-connection",
        &["--project-id", COORD_PROJECT, "--connection", CONNECTION_KM],
    );
    let elements = list(&resolved, "elements");
    assert_eq!(elements.len(), 1);
    let source = list(&elements[0], "source_mm");
    let parent = list(&elements[0], "parent_mm");
    // The source plate sits at (500, 250, 0); a quarter turn about Z maps it to
    // (-250, 500, 0), and the connection adds (0, 10000, 0).
    assert!((source[0].float().unwrap() - 500.0).abs() < 1e-9);
    assert!((source[1].float().unwrap() - 250.0).abs() < 1e-9);
    assert!((parent[0].float().unwrap() + 250.0).abs() < 1e-6);
    assert!((parent[1].float().unwrap() - 10500.0).abs() < 1e-6);
    assert!(parent[2].float().unwrap().abs() < 1e-9);
}

#[test]
fn the_closure_is_retained_without_the_source_and_across_compaction() {
    let f = Fixture::new();
    let km_revision = f.source_store("km", KM_PROJECT, KM_WORKSPACE, KM_SECTION, "KM");
    let kmd_revision = f.source_store("kmd", KMD_PROJECT, KMD_WORKSPACE, KMD_SECTION, "KMD");
    for (connection, project, _workspace, revision, store) in [
        (
            CONNECTION_KM,
            KM_PROJECT,
            KM_WORKSPACE,
            km_revision.clone(),
            "km",
        ),
        (
            CONNECTION_KMD,
            KMD_PROJECT,
            KMD_WORKSPACE,
            kmd_revision.clone(),
            "kmd",
        ),
    ] {
        f.ok(
            "coord",
            "connect-project",
            &Fixture::with_ids(
                COORD_PROJECT,
                COORD_WORKSPACE,
                &[
                    "--source-store",
                    f.shared.root.join(store).to_str().unwrap(),
                    "--source-project",
                    project,
                    "--source-revision",
                    &revision,
                    "--connection-id",
                    connection,
                    "--label",
                    connection,
                    "--translation",
                    "1000,0,0",
                    "--orientation",
                    "0,0,0,1",
                ],
            ),
        );
    }

    // The coordination store holds verified local copies: with both source stores gone the
    // pins still resolve completely (PC-REV-001, PC-KEEP-001).
    fs::rename(f.shared.root.join("km"), f.shared.root.join("km-away")).unwrap();
    fs::rename(f.shared.root.join("kmd"), f.shared.root.join("kmd-away")).unwrap();
    let shown = f.ok("coord", "show-project", &["--project-id", COORD_PROJECT]);
    for status in list(&shown, "connections") {
        assert!(flag(status, "complete"));
    }
    f.ok(
        "coord",
        "resolve-connection",
        &["--project-id", COORD_PROJECT, "--connection", CONNECTION_KM],
    );

    // Compaction of the coordination store keeps the live pinned closure of the head.
    f.ok(
        "coord",
        "compact-history",
        &["--keep-head", "--yes", "--no-backup", "--no-workspaces"],
    );
    let shown = f.ok("coord", "show-project", &["--project-id", COORD_PROJECT]);
    for status in list(&shown, "connections") {
        assert!(flag(status, "complete"), "{status:?}");
    }
    f.ok(
        "coord",
        "resolve-connection",
        &[
            "--project-id",
            COORD_PROJECT,
            "--connection",
            CONNECTION_KMD,
        ],
    );
    fs::rename(f.shared.root.join("km-away"), f.shared.root.join("km")).unwrap();
    fs::rename(f.shared.root.join("kmd-away"), f.shared.root.join("kmd")).unwrap();

    // Removing a connection drops it from the head only; the bytes of older retained
    // revisions stay pinned until history itself is retired.
    f.ok(
        "coord",
        "remove-connection",
        &Fixture::with_ids(
            COORD_PROJECT,
            COORD_WORKSPACE,
            &["--connection", CONNECTION_KMD],
        ),
    );
    let shown = f.ok("coord", "show-project", &["--project-id", COORD_PROJECT]);
    let connections = list(at(&shown, &["descriptor"]), "connections");
    assert_eq!(connections.len(), 1);
    assert_eq!(text(&connections[0], "id"), CONNECTION_KM);

    // Tampered pinned bytes refuse the whole revision with INTEGRITY_ERROR (PC-REV-001).
    let shown = f.ok("coord", "show-project", &["--project-id", COORD_PROJECT]);
    let digest = text(
        &list(
            &list(at(&shown, &["descriptor"]), "connections")[0],
            "layers",
        )[0],
        "digest",
    );
    let blob = f
        .shared
        .root
        .join("coord")
        .join("blobs")
        .join(&digest[..2])
        .join(&digest);
    let original = fs::read(&blob).unwrap();
    fs::write(&blob, b"tampered bytes").unwrap();
    let (code, body) = f.run("coord", "show-project", &["--project-id", COORD_PROJECT]);
    assert_eq!(code, 2, "{body:?}");
    assert_eq!(text(&body, "code"), "INTEGRITY_ERROR");
    fs::write(&blob, original).unwrap();
    let (code, body) = f.run("coord", "show-project", &["--project-id", COORD_PROJECT]);
    assert_eq!(code, 0, "{body:?}");
}

#[test]
fn sections_edit_their_own_stores_only() {
    let f = Fixture::new();
    f.ok(
        "km",
        "set-project-metadata",
        &Fixture::with_ids(
            KM_PROJECT,
            KM_WORKSPACE,
            &["--code", "2401-DEMO", "--name", "Demo warehouse"],
        ),
    );
    let km_revision = f.source_store("km", KM_PROJECT, KM_WORKSPACE, KM_SECTION, "KM");
    // An edit in the KMD project leaves the KM project store byte-stable (PC-OWN-001/002:
    // separate writable sections; shared metadata changes only through the explicit
    // commands, which no modelling command is).
    f.source_store("kmd", KMD_PROJECT, KMD_WORKSPACE, KMD_SECTION, "KMD");
    assert_eq!(f.head("km"), km_revision);
    let shown = f.ok("km", "show-project", &["--project-id", KM_PROJECT]);
    assert_eq!(text(at(&shown, &["descriptor"]), "code"), "2401-DEMO");
}
