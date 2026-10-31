// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic projects, MPL-2.0. Both projects author KM and KMD through the
// same steel module; the section scopes keep them apart (F10 0.2 DV-SCOPE-001..004).

//! U19 end to end through the real `beamcraft` binary: sectioned projects, authorized
//! workspace scopes, and the refusals that keep other sections, zones and shared layers
//! untouched.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use crate::support::{self, at, text};
use beamcraft_json::{Json, Limits};
use std::process::Command;

const ACTOR: &str = "019a7bf0-0000-7000-8000-00000000a001";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        Self {
            shared: support::StoreFixture::new("u19-scopes"),
        }
    }

    /// Runs a start/project command without the implicit `--store`.
    fn raw(&self, args: &[&str]) -> (i32, Json) {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .output()
            .unwrap();
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
        (output.status.code().unwrap(), body)
    }

    fn raw_ok(&self, args: &[&str]) -> Json {
        let (code, body) = self.raw(args);
        assert_eq!(code, 0, "{args:?}: {body:?}");
        body
    }

    fn raw_rejected(&self, args: &[&str]) -> Json {
        let (code, body) = self.raw(args);
        assert_eq!(code, 2, "{args:?}: {body:?}");
        body
    }

    /// Creates one synthetic project and returns its store path.
    fn project(&self, name: &str, code: &str, sections: &str) -> String {
        let parent = self.shared.root.join("models");
        std::fs::create_dir_all(&parent).unwrap();
        let config = self.shared.root.join("config");
        let reply = self.raw_ok(&[
            "create-project",
            "--parent",
            parent.to_str().unwrap(),
            "--name",
            name,
            "--code",
            code,
            "--sections",
            sections,
            "--actor-id",
            ACTOR,
            "--config-dir",
            config.to_str().unwrap(),
        ]);
        text(at(&reply, &["project"]), "path")
    }

    fn run_in(&self, store: &str, command: &str, extra: &[&str]) -> (i32, Json) {
        support::StoreFixture::run_in(store, command, extra)
    }

    fn ok_in(&self, store: &str, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run_in(store, command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    }

    /// Asserts a rejection and returns the error JSON for code/rule assertions.
    fn refused_in(&self, store: &str, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run_in(store, command, extra);
        assert_eq!(code, 2, "{command} {extra:?}: {body:?}");
        body
    }

    fn workspace(
        &self,
        store: &str,
        dir: &str,
        code: &str,
        section: &str,
        zone: Option<&str>,
        shared: bool,
    ) {
        let mut args = vec![
            "create-workspace",
            "--store",
            store,
            "--workspace-dir",
            dir,
            "--scope-code",
            code,
            "--scope-section",
            section,
        ];
        if let Some(zone) = zone {
            args.extend(["--scope-zone", zone]);
        }
        if shared {
            args.extend(["--scope-shared", "true"]);
        }
        self.raw_ok(&args);
    }

    fn publish(&self, store: &str, dir: &str) -> Json {
        self.ok_in(store, "publish-workspace", &["--workspace-dir", dir])
    }

    /// The SHA-256 blob file of one layer of the revision a publish reply carries.
    fn layer_of(&self, store: &str, revision: &Json, name: &str) -> Vec<u8> {
        let layers = at(revision, &["revision", "manifest", "layers"])
            .array()
            .unwrap();
        let digest = layers
            .iter()
            .find(|l| text(l, "name") == name)
            .unwrap_or_else(|| panic!("{name} missing from the revision layers"))
            .get("digest")
            .unwrap()
            .string()
            .unwrap()
            .to_owned();
        fs::read(
            std::path::Path::new(store)
                .join("blobs")
                .join(&digest[..2])
                .join(&digest),
        )
        .unwrap()
    }
}

use std::fs;

/// The F10 rule of the first diagnostic of a rejection.
fn rule(json: &Json) -> String {
    text(&at(json, &["diagnostics"]).array().unwrap()[0], "rule_id")
}

fn plate_args<'a>(new_id: &'a str, name: &'a str) -> Vec<&'a str> {
    vec![
        "--width",
        "1000",
        "--length",
        "500",
        "--thickness",
        "10",
        "--new-id",
        new_id,
        "--name",
        name,
    ]
}

#[test]
fn the_layout_names_one_layer_per_scope_and_the_first_proof_runs_both_sections() {
    let f = Fixture::new();
    // First proof, project one: KM and KMD in one store, both authored through the steel
    // module, each behind its own authorized workspace scope.
    let store = f.project("Shared", "2401-DEMO", "KM,K1:KMD,K2:KMD");
    let created = f.ok_in(&store, "open-revision", &[]);
    let layers: Vec<String> = at(&created, &["layers"])
        .array()
        .unwrap()
        .iter()
        .map(|l| text(l, "name"))
        .collect();
    for expected in [
        "2401-DEMO_Grids.usda",
        "2401-DEMO_KM.usda",
        "2401-DEMO_K1_KMD.usda",
        "2401-DEMO_K2_KMD.usda",
        "derived.usda",
    ] {
        assert!(layers.contains(&expected.to_owned()), "{layers:?}");
    }
    assert!(!layers.iter().any(|l| l == "authored.usda"), "{layers:?}");

    // First proof, project two: one KMD-only project, same steel module.
    let single = f.project("Independent", "2402-DEMO", "KMD");
    let dir = f.shared.root.join("w-single");
    f.workspace(
        &single,
        dir.to_str().unwrap(),
        "2402-DEMO",
        "KMD",
        None,
        false,
    );
    let reply = f.ok_in(
        dir.to_str().unwrap(),
        "create-plate",
        &plate_args("019a7bf0-0000-7000-8000-0000000000b1", "Solo plate"),
    );
    assert_eq!(text(&reply, "outcome"), "plate_created");
    let published = f.publish(&single, dir.to_str().unwrap());
    let layer = f.layer_of(&single, &published, "2402-DEMO_KMD.usda");
    assert!(String::from_utf8_lossy(&layer).contains("Solo plate"));
}

#[test]
fn an_authorized_scope_publishes_inside_itself_and_preserves_the_rest() {
    let f = Fixture::new();
    let store = f.project("Shared", "2401-DEMO", "KM,K1:KMD,K2:KMD");
    // DV-SCOPE-002 positive: the steel module authors the KM section through its own scope...
    let wkm = f.shared.root.join("w-km");
    f.workspace(
        &store,
        wkm.to_str().unwrap(),
        "2401-DEMO",
        "KM",
        None,
        false,
    );
    let km = "019a7bf0-0000-7000-8000-0000000000a2";
    f.ok_in(
        wkm.to_str().unwrap(),
        "create-plate",
        &plate_args(km, "KM plate"),
    );
    let km_published = f.publish(&store, wkm.to_str().unwrap());

    // ...and the KMD/K1 workspace, forked after the KM publication, authors and edits the
    // KMD section with the same module.
    let wkmd = f.shared.root.join("w-kmd");
    f.workspace(
        &store,
        wkmd.to_str().unwrap(),
        "2401-DEMO",
        "KMD",
        Some("K1"),
        false,
    );
    let kmd = "019a7bf0-0000-7000-8000-0000000000a3";
    f.ok_in(
        wkmd.to_str().unwrap(),
        "create-plate",
        &plate_args(kmd, "KMD plate"),
    );
    f.ok_in(
        wkmd.to_str().unwrap(),
        "update-element",
        &["--id", kmd, "--name", "KMD plate renamed"],
    );

    // DV-SCOPE-001 positive: the authorized KMD/K1 workspace publishes within its scope.
    let kmd_published = f.publish(&store, wkmd.to_str().unwrap());
    let kmd_layer = f.layer_of(&store, &kmd_published, "2401-DEMO_K1_KMD.usda");
    assert!(String::from_utf8_lossy(&kmd_layer).contains("KMD plate renamed"));
    // DV-SCOPE-003 positive: the KM section, the other zone, the shared layer and the root
    // composition are preserved byte for byte.
    for name in [
        "2401-DEMO_KM.usda",
        "2401-DEMO_K2_KMD.usda",
        "2401-DEMO_Grids.usda",
        "root.usda",
    ] {
        assert_eq!(
            f.layer_of(&store, &km_published, name),
            f.layer_of(&store, &kmd_published, name),
            "{name} changed"
        );
    }
}

#[test]
fn the_same_module_cannot_edit_another_section_or_zone() {
    let f = Fixture::new();
    let store = f.project("Shared", "2401-DEMO", "KM,K1:KMD,K2:KMD");
    // The workspaces fork in publication order: a draft is fast-forward only.
    let wkm = f.shared.root.join("w-km");
    f.workspace(
        &store,
        wkm.to_str().unwrap(),
        "2401-DEMO",
        "KM",
        None,
        false,
    );
    let km = "019a7bf0-0000-7000-8000-0000000000a4";
    f.ok_in(
        wkm.to_str().unwrap(),
        "create-plate",
        &plate_args(km, "KM plate"),
    );
    f.publish(&store, wkm.to_str().unwrap());

    let wkmd = f.shared.root.join("w-kmd");
    f.workspace(
        &store,
        wkmd.to_str().unwrap(),
        "2401-DEMO",
        "KMD",
        Some("K1"),
        false,
    );
    let kmd = "019a7bf0-0000-7000-8000-0000000000a5";
    f.ok_in(
        wkmd.to_str().unwrap(),
        "create-plate",
        &plate_args(kmd, "KMD plate"),
    );
    f.publish(&store, wkmd.to_str().unwrap());

    let wkmd2 = f.shared.root.join("w-kmd2");
    f.workspace(
        &store,
        wkmd2.to_str().unwrap(),
        "2401-DEMO",
        "KMD",
        Some("K2"),
        false,
    );

    // DV-SCOPE-002 negative, other section: the same steel module cannot edit a KM element
    // from the KMD workspace; a stronger opinion over it is still an edit.
    let refused = f.refused_in(
        wkmd.to_str().unwrap(),
        "update-element",
        &["--id", km, "--name", "Smuggled"],
    );
    assert_eq!(text(&refused, "code"), "FOREIGN_READ_ONLY");
    assert_eq!(rule(&refused), "DV-SCOPE-002");

    // DV-SCOPE-002 negative, other zone: the K2 workspace cannot reach this K1 element.
    let refused = f.refused_in(
        wkmd2.to_str().unwrap(),
        "update-element",
        &["--id", kmd, "--name", "Other zone"],
    );
    assert_eq!(text(&refused, "code"), "FOREIGN_READ_ONLY");
    assert_eq!(rule(&refused), "DV-SCOPE-002");

    // A delete is an edit like any other.
    let refused = f.refused_in(wkmd.to_str().unwrap(), "delete-element", &["--id", km]);
    assert_eq!(text(&refused, "code"), "FOREIGN_READ_ONLY");
}

#[test]
fn a_claimed_scope_change_cannot_smuggle_publication() {
    let f = Fixture::new();
    let store = f.project("Shared", "2401-DEMO", "KM,K1:KMD,K2:KMD");
    let wkmd = f.shared.root.join("w-kmd");
    f.workspace(
        &store,
        wkmd.to_str().unwrap(),
        "2401-DEMO",
        "KMD",
        Some("K1"),
        false,
    );
    let kmd = "019a7bf0-0000-7000-8000-0000000000a6";
    f.ok_in(
        wkmd.to_str().unwrap(),
        "create-plate",
        &plate_args(kmd, "KMD plate"),
    );

    // DV-SCOPE-001 negative: the recorded authorization, not a client claim, decides what
    // publishes. Re-labelling the workspace's scope as KM leaves the unsent KMD work outside
    // the claimed scope, and the publisher refuses it.
    let meta = wkmd.join("workspace.json");
    let meta_text = fs::read_to_string(&meta).unwrap();
    fs::write(
        &meta,
        meta_text.replace("scope_section=KMD", "scope_section=KM"),
    )
    .unwrap();
    let refused = f.refused_in(
        &store,
        "publish-workspace",
        &["--workspace-dir", wkmd.to_str().unwrap()],
    );
    assert_eq!(text(&refused, "code"), "PERMISSION_DENIED");
    assert_eq!(rule(&refused), "DV-SCOPE-001");

    // A malformed scope record is not usable either.
    fs::write(
        &meta,
        meta_text.replace("scope_section=KMD", "scope_section=K_M"),
    )
    .unwrap();
    let refused = f.refused_in(
        wkmd.to_str().unwrap(),
        "create-plate",
        &plate_args("019a7bf0-0000-7000-8000-0000000000a7", "X"),
    );
    assert_eq!(text(&refused, "code"), "PERMISSION_DENIED");
}

#[test]
fn a_sectioned_store_answers_only_to_an_authorized_workspace() {
    let f = Fixture::new();
    let store = f.project("Shared", "2401-DEMO", "KM,K1:KMD,K2:KMD");
    // No workspace, no scope: the published store refuses every edit.
    let refused = f.refused_in(
        &store,
        "create-plate",
        &plate_args("019a7bf0-0000-7000-8000-0000000000a8", "X"),
    );
    assert_eq!(text(&refused, "code"), "PERMISSION_DENIED");
    assert_eq!(rule(&refused), "DV-SCOPE-001");

    // A workspace without an authorized scope refuses too.
    let bare = f.shared.root.join("w-bare");
    f.raw_ok(&[
        "create-workspace",
        "--store",
        &store,
        "--workspace-dir",
        bare.to_str().unwrap(),
    ]);
    let refused = f.refused_in(
        bare.to_str().unwrap(),
        "create-plate",
        &plate_args("019a7bf0-0000-7000-8000-0000000000a9", "X"),
    );
    assert_eq!(text(&refused, "code"), "PERMISSION_DENIED");

    // The authorization is explicit and checked against the layout: a scope whose layers the
    // store does not contain is refused.
    let refused = f.raw_rejected(&[
        "create-workspace",
        "--store",
        &store,
        "--workspace-dir",
        f.shared.root.join("w-none").to_str().unwrap(),
        "--scope-code",
        "OTHER",
        "--scope-section",
        "KM",
    ]);
    assert_eq!(text(&refused, "code"), "NOT_FOUND");

    // A scope on a classic 0.1 store is refused: nothing infers sections from a store.
    let classic = f.shared.root.join("classic-store");
    std::fs::create_dir(&classic).unwrap();
    f.ok_in(
        classic.to_str().unwrap(),
        "create-plate",
        &plate_args("019a7bf0-0000-7000-8000-0000000000aa", "Classic"),
    );
    let refused = f.raw_rejected(&[
        "create-workspace",
        "--store",
        classic.to_str().unwrap(),
        "--workspace-dir",
        f.shared.root.join("w-classic").to_str().unwrap(),
        "--scope-code",
        "2401-DEMO",
        "--scope-section",
        "KM",
    ]);
    assert_eq!(text(&refused, "code"), "INVALID_ARGUMENT");
}

#[test]
fn shared_grids_answer_only_to_authorized_shared_commands() {
    let f = Fixture::new();
    let store = f.project("Shared", "2401-DEMO", "KM,K1:KMD,K2:KMD");
    // DV-SCOPE-004 positive: the separately authorized shared command updates the grids...
    let wshared = f.shared.root.join("w-shared");
    f.workspace(
        &store,
        wshared.to_str().unwrap(),
        "2401-DEMO",
        "KM",
        None,
        true,
    );
    let grid = f.ok_in(
        wshared.to_str().unwrap(),
        "create-grid",
        &["--name", "Main", "--u-expression", "0 3*6000"],
    );
    assert_eq!(text(&grid, "outcome"), "grid_created");
    let published = f.publish(&store, wshared.to_str().unwrap());
    let grids_layer = f.layer_of(&store, &published, "2401-DEMO_Grids.usda");
    assert!(String::from_utf8_lossy(&grids_layer).contains("Main"));
    // ...and the section layers stay out of the shared command's reach.
    let km_layer = f.layer_of(&store, &published, "2401-DEMO_KM.usda");
    assert!(!String::from_utf8_lossy(&km_layer).contains("Main"));

    // A section workspace, forked after the shared publication, cannot author the common
    // grids (DV-SCOPE-004 negative)...
    let wkmd = f.shared.root.join("w-kmd");
    f.workspace(
        &store,
        wkmd.to_str().unwrap(),
        "2401-DEMO",
        "KMD",
        Some("K1"),
        false,
    );
    let refused = f.refused_in(
        wkmd.to_str().unwrap(),
        "create-grid",
        &["--name", "Sneaky", "--u-expression", "0 6000"],
    );
    assert_eq!(text(&refused, "code"), "PERMISSION_DENIED");
    assert_eq!(rule(&refused), "DV-SCOPE-004");

    // ...but still publishes its own scope beside the shared work.
    let kmd = "019a7bf0-0000-7000-8000-0000000000ab";
    f.ok_in(
        wkmd.to_str().unwrap(),
        "create-plate",
        &plate_args(kmd, "KMD plate"),
    );
    let published = f.publish(&store, wkmd.to_str().unwrap());
    let layer = f.layer_of(&store, &published, "2401-DEMO_K1_KMD.usda");
    assert!(String::from_utf8_lossy(&layer).contains("KMD plate"));
    // The shared layer is preserved byte for byte by the section publication.
    assert_eq!(
        f.layer_of(&store, &published, "2401-DEMO_Grids.usda"),
        grids_layer,
    );
}
