// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic libraries, environments and sections that follow no national
// standard, MPL-2.0. Every library and environment is written by the tests from plain text.

//! #628 project portability, end to end through the real `beamcraft` binary: the three states
//! of a project's own environment copy are told apart. A plain project without an environment
//! folder is a normal state and warns nowhere; a portable project whose self-contained copy
//! reads answers from it; and a copy whose `environment/` folder survives without its
//! `environment.usda` is demonstrably incomplete — only `create-project --environment` makes
//! the folder — so the library replies report `environment_lost` instead of silently falling
//! back to firm and neutral. Definitions the model pinned before the loss keep their values,
//! and a copy whose `environment.usda` survives with a missing library still refuses loudly.
//! Every run names its own `BEAMCRAFT_CONFIG_DIR`, so the user's real application data is
//! never read or written.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{self, at, list, text};

use beamcraft_json::{Json, Limits};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const ACTOR: &str = "0199e7a0-0000-7000-8000-000000000070";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("project-environment-warning-tests"),
        };
        fs::create_dir_all(fixture.shared.root.join("config/libraries")).unwrap();
        fs::create_dir(fixture.shared.root.join("models")).unwrap();
        fixture
    }
    fn path(&self, name: &str) -> String {
        self.shared.root.join(name).to_str().unwrap().to_owned()
    }
    fn raw(&self, args: &[&str]) -> (i32, Json) {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .env("BEAMCRAFT_CONFIG_DIR", self.path("config"))
            .output()
            .unwrap();
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
        (output.status.code().unwrap(), body)
    }
    fn ok(&self, args: &[&str]) -> Json {
        let (code, body) = self.raw(args);
        assert_eq!(code, 0, "{args:?}: {body:?}");
        body
    }
    fn code(&self, args: &[&str]) -> String {
        let (code, body) = self.raw(args);
        assert_eq!(code, 2, "{args:?}: {body:?}");
        text(&body, "code")
    }
    /// Installs the synthetic `acme` environment into the application data: one profile
    /// library with `X-1` (200 mm) and `AC-ONLY` (210 mm), no fastener or material libraries.
    fn install_acme(&self) {
        let dir = self.shared.root.join("config/environments/acme");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("environment.usda"),
            "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {\n        string \"bim:environment:id\" = \"acme\"\n        string \"bim:environment:title\" = \"Acme\"\n        string[] \"bim:environment:libraries\" = [\"acme.usda\"]\n    }\n)\n",
        )
        .unwrap();
        let entry = |prim: &str, designation: &str, height: f64| {
            format!(
                "        def Scope \"{prim}\"\n        {{\nstring steel:profile:designation = \"{designation}\"\ntoken steel:profile:family = \"i_shape\"\ndouble steel:profile:hMm = {height}\ndouble steel:profile:bMm = 150\ndouble steel:profile:twMm = 7\ndouble steel:profile:tfMm = 10\ndouble steel:profile:rMm = 15\n        }}\n"
            )
        };
        fs::write(
            dir.join("acme.usda"),
            format!(
                "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {{\n        string \"bim:library:id\" = \"acme-lib\"\n        string \"bim:library:source\" = \"Contributor-original synthetic values for tests\"\n        string \"bim:library:licence\" = \"MPL-2.0\"\n    }}\n)\ndef Scope \"Library\"\n{{\n    def Scope \"Profiles\"\n    {{\n{}{}    }}\n}}\n",
                entry("EX", "X-1", 200.0),
                entry("EA", "AC-ONLY", 210.0)
            ),
        )
        .unwrap();
    }
    /// Writes one firm library file into the application data's `libraries/`: `FIRM-ONLY`
    /// at 280 mm.
    fn firm(&self) -> String {
        let body = "        def Scope \"EF\"\n        {\nstring steel:profile:designation = \"FIRM-ONLY\"\ntoken steel:profile:family = \"i_shape\"\ndouble steel:profile:hMm = 280\ndouble steel:profile:bMm = 150\ndouble steel:profile:twMm = 7\ndouble steel:profile:tfMm = 10\ndouble steel:profile:rMm = 15\n        }\n";
        let path = self.shared.root.join("config/libraries/mine.usda");
        fs::write(
            &path,
            format!(
                "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {{\n        string \"bim:library:id\" = \"firm-lib\"\n    }}\n)\ndef Scope \"Library\"\n{{\n    def Scope \"Profiles\"\n    {{\n{body}    }}\n}}\n"
            ),
        )
        .unwrap();
        path.to_str().unwrap().to_owned()
    }
    /// Creates a project named `name` with the given extra options and answers its store path.
    fn create(&self, name: &str, extra: &[&str]) -> String {
        let models = self.path("models");
        let mut args: Vec<&str> = vec![
            "create-project",
            "--parent",
            &models,
            "--name",
            name,
            "--actor-id",
            ACTOR,
        ];
        args.extend_from_slice(extra);
        text(at(&self.ok(&args), &["project"]), "path")
    }
    /// The height the name resolves to in the store, after it was defined by its first use.
    fn profile_height(&self, store: &str, name: &str) -> f64 {
        let defined = self.ok(&["definitions", "--store", store, "--name", name]);
        let found = list(&defined, "definitions");
        assert_eq!(found.len(), 1, "{name}: {found:?}");
        at(&found[0], &["parameters_mm", "h"]).float().unwrap()
    }
    fn member(&self, store: &str, profile: &str) -> Json {
        self.ok(&[
            "create-member",
            "--store",
            store,
            "--kind",
            "beam",
            "--start",
            "0,0,0",
            "--end",
            "6000,0,0",
            "--profile",
            profile,
            "--material",
            "Steel",
        ])
    }
    /// The layers of the project chain, strongest first, as the listing reports them.
    fn layers(&self, store: &str) -> Vec<String> {
        let listed = self.ok(&["list-library", "--project", store]);
        list(&listed, "libraries")
            .iter()
            .map(|s| text(s, "layer"))
            .collect()
    }
}

/// The reported path of the lost copy, kept as a path so comparisons are
/// separator-agnostic: a Windows run reports backslashes.
fn lost_of(listed: &Json) -> PathBuf {
    PathBuf::from(text(at(listed, &["environment_lost"]), "path"))
}

#[test]
fn the_three_copy_states_are_told_apart_in_the_library_replies() {
    let f = Fixture::new();
    f.install_acme();
    f.firm();

    // A plain project, created without an environment: no folder, no warning, the chain is
    // firm and neutral alone — a normal state, not a loss.
    let plain = f.create("Plain", &[]);
    assert!(!Path::new(&plain).join("environment").exists());
    let listed = f.ok(&["list-library", "--project", &plain]);
    assert_eq!(f.layers(&plain), ["firm", "neutral"]);
    assert!(
        listed.get("environment_lost").is_err(),
        "a plain project warns nowhere: {listed:?}"
    );
    assert!(listed.get("environment").is_err());

    // A portable project: the self-contained copy is the project layer and answers first.
    let seeded = f.create("Seeded", &["--environment", "acme"]);
    assert_eq!(f.layers(&seeded), ["project", "firm", "neutral"]);
    let listed = f.ok(&["list-library", "--project", &seeded]);
    assert!(
        listed.get("environment_lost").is_err(),
        "a reading copy warns nowhere: {listed:?}"
    );
    assert_eq!(text(at(&listed, &["environment"]), "id"), "acme");

    // The recognizable loss: the copy's folder survives, its environment.usda is gone. The
    // folder proves an environment was chosen at creation, so the fallback is reported.
    fs::remove_file(Path::new(&seeded).join("environment/environment.usda")).unwrap();
    let listed = f.ok(&["list-library", "--project", &seeded]);
    assert_eq!(f.layers(&seeded), ["firm", "neutral"]);
    assert!(
        lost_of(&listed).ends_with("environment/environment.usda"),
        "{}",
        lost_of(&listed).display()
    );
    assert_eq!(
        text(at(&listed, &["environment_lost"]), "message"),
        "the project's environment copy has lost its environment.usda; profile, fastener and material names resolve from the firm libraries and the built-in neutral environment, not the environment the project was created with"
    );

    // `show-library` carries the same warning, and a designation the copy answered is simply
    // absent now: the fallback substitutes no other section under that name.
    let shown = f.ok(&[
        "show-library",
        "--project",
        &seeded,
        "--designation",
        "FIRM-ONLY",
    ]);
    assert_eq!(text(at(&shown, &["library"]), "id"), "firm-lib");
    assert!(lost_of(&shown).ends_with("environment/environment.usda"));
    assert_eq!(
        f.code(&["show-library", "--project", &seeded, "--designation", "X-1"]),
        "NOT_FOUND"
    );
}

#[test]
fn pinned_definitions_keep_their_values_after_the_copy_is_lost() {
    let f = Fixture::new();
    f.install_acme();
    f.firm();
    let store = f.create("Pinned", &["--environment", "acme"]);
    f.member(&store, "X-1");
    assert_eq!(f.profile_height(&store, "X-1"), 200.0);

    // The copy is lost. The definition the model pinned by content stays complete: a member
    // of the same name resolves from the model layer, before the chain is ever consulted.
    fs::remove_file(Path::new(&store).join("environment/environment.usda")).unwrap();
    f.member(&store, "X-1");
    assert_eq!(
        f.profile_height(&store, "X-1"),
        200.0,
        "the pinned definition answers without the library files"
    );
    // The chain reports the loss wherever the chain itself is asked.
    let listed = f.ok(&["list-library", "--project", &store]);
    assert!(lost_of(&listed).ends_with("environment/environment.usda"));
}

#[test]
fn a_copy_that_survives_with_a_missing_library_still_refuses_loudly() {
    let f = Fixture::new();
    f.install_acme();
    let store = f.create("Hollow", &["--environment", "acme"]);
    // The copy's environment.usda survives, its library file is gone: that copy cannot be
    // read without silently changing what a designation answers, so it refuses the chain —
    // no warning-with-fallback here.
    fs::remove_file(Path::new(&store).join("environment/acme.usda")).unwrap();
    let (code, body) = f.raw(&["list-library", "--project", &store]);
    assert_eq!(code, 2, "{body:?}");
    assert_eq!(text(&body, "code"), "NOT_FOUND");
    assert!(
        body.get("environment_lost").is_err(),
        "the refusal is the answer, no warning beside it: {body:?}"
    );
}
