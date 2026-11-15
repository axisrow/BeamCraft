// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic libraries and sections that follow no national standard,
// MPL-2.0. Every library and environment is written by the tests from plain text.

//! The application-data layout and the settings-selected active environment, end to end through
//! the real `beamcraft` binary: `create-project` without `--environment` seeds the environment
//! the settings name, the option overrides it, a missing or unreadable choice refuses without
//! leaving a folder, and an environment updated in the application data never touches the copy a
//! created project already carries. Every run names its own `--config-dir`, so the user's real
//! settings are never read or written.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support;

use support::{at, list, text};

use beamcraft_json::{Json, Limits};
use std::fs;
use std::path::Path;
use std::process::Command;

const ACTOR: &str = "0199e7a0-0000-7000-8000-0000000000d1";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("appdata-settings-tests"),
        };
        fs::create_dir(fixture.shared.root.join("models")).unwrap();
        fixture
    }
    fn config(&self) -> String {
        self.shared.root.join("config").to_str().unwrap().to_owned()
    }
    fn path(&self, name: &str) -> String {
        self.shared.root.join(name).to_str().unwrap().to_owned()
    }
    fn raw(&self, args: &[&str]) -> (i32, Json) {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .env_remove("BEAMCRAFT_CONFIG_DIR")
            .output()
            .unwrap();
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
        (output.status.code().unwrap(), body)
    }
    /// Runs a start command with this fixture's configuration directory.
    fn with_config(&self, args: &[&str]) -> (i32, Json) {
        let config = self.config();
        let mut all = args.to_vec();
        all.extend(["--config-dir", &config]);
        self.raw(&all)
    }
    fn ok(&self, args: &[&str]) -> Json {
        let (code, body) = self.with_config(args);
        assert_eq!(code, 0, "{args:?}: {body:?}");
        body
    }
    fn code(&self, args: &[&str]) -> String {
        let (code, body) = self.with_config(args);
        assert_eq!(code, 2, "{args:?}: {body:?}");
        text(&body, "code")
    }
    /// Puts a one-entry user environment with the given `id` into the application data and names
    /// it the settings' active environment. The entry `AC-BEAM` is a rectangle `height` x 150,
    /// so the height decides every mass.
    fn install_environment(&self, id: &str, height: f64) {
        let dir = self.shared.root.join("config/environments").join(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("environment.usda"),
            format!(
                "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {{\n        string \"bim:environment:id\" = \"{id}\"\n        string \"bim:environment:title\" = \"Acme\"\n        string[] \"bim:environment:libraries\" = [\"acme.usda\"]\n    }}\n)\n"
            ),
        )
        .unwrap();
        fs::write(
            dir.join("acme.usda"),
            format!(
                "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {{\n        string \"bim:library:id\" = \"acme-{id}\"\n        string \"bim:library:source\" = \"Contributor-original synthetic values for tests\"\n        string \"bim:library:licence\" = \"MPL-2.0\"\n    }}\n)\ndef Scope \"Library\"\n{{\n    def Scope \"Profiles\"\n    {{\n        def Scope \"AC_BEAM\"\n        {{\nstring steel:profile:designation = \"AC-BEAM\"\ntoken steel:profile:family = \"rectangle\"\ndouble steel:profile:hMm = {height}\ndouble steel:profile:bMm = 150\ndouble steel:profile:areaMm2 = {}\n        }}\n    }}\n}}\n",
                height * 150.0
            ),
        )
        .unwrap();
        self.set_settings_environment(id);
    }
    /// Writes a minimal version-1 settings file that names `id` as the active environment.
    fn set_settings_environment(&self, id: &str) {
        let config = self.shared.root.join("config");
        fs::create_dir_all(&config).unwrap();
        fs::write(
            config.join("settings.json"),
            format!(
                "{{\"environment\":\"{id}\",\"models_dir\":null,\"profile\":null,\"recent\":[],\"skip_launcher\":false,\"version\":1}}"
            ),
        )
        .unwrap();
    }
    /// Writes settings bytes that no version-1 reader accepts.
    fn damage_settings(&self) {
        let config = self.shared.root.join("config");
        fs::create_dir_all(&config).unwrap();
        fs::write(config.join("settings.json"), b"this is not settings").unwrap();
    }
    /// Creates a project named `name` with no environment option and returns the reply.
    fn create(&self, name: &str) -> (i32, Json) {
        let models = self.path("models");
        self.with_config(&["create-project", "--parent", &models, "--name", name])
    }
    /// Places one 6 m beam of the fixture's `AC-BEAM` entry in `store` and returns its mass in
    /// kg: the entry is defined by its first use, from the project's own environment copy.
    fn beam_mass(&self, store: &str) -> f64 {
        let member = self.raw(&[
            "create-member",
            "--store",
            store,
            "--start",
            "0,0,0",
            "--end",
            "6000,0,0",
            "--profile",
            "AC-BEAM",
            "--material",
            "Steel",
        ]);
        assert_eq!(member.0, 0, "{:?}", member.1);
        let mass = at(&member.1, &["member", "quantities", "mass"]);
        assert_eq!(text(mass, "status"), "current");
        at(mass, &["kg"]).float().unwrap()
    }
}

/// The mass of a 6 m beam of the fixture's `AC-BEAM` entry of the given height: the declared
/// area `height x 150` is a rectangle's exact area, so no warning fires and the mass is exact.
fn expected_mass(height: f64) -> f64 {
    height * 150.0 * 1.0e-6 * 6.0 * 7850.0
}

#[test]
fn create_project_seeds_the_settings_active_environment_without_the_option() {
    let f = Fixture::new();
    f.install_environment("acme", 300.0);
    let (code, created) = f.create("From Settings");
    assert_eq!(code, 0, "{created:?}");
    let seeded = at(&created, &["seeded"]);
    assert_eq!(text(seeded, "environment"), "acme");
    assert_eq!(text(seeded, "environment_source"), "settings");
    assert_eq!(at(seeded, &["materials"]).unsigned().unwrap(), 2);
    let store = text(at(&created, &["project"]), "path");
    assert!(
        Path::new(&store)
            .join("environment/environment.usda")
            .is_file(),
        "the project carries its copy"
    );
    let kg = f.beam_mass(&store);
    assert!(
        (kg - expected_mass(300.0)).abs() < 0.001,
        "{kg} kg, expected the catalogue area mass"
    );
}

#[test]
fn the_environment_option_overrides_the_settings_choice() {
    let f = Fixture::new();
    f.install_environment("acme", 300.0);
    let models = f.path("models");
    let created = f.ok(&[
        "create-project",
        "--parent",
        &models,
        "--name",
        "Overridden",
        "--environment",
        "blank",
    ]);
    let seeded = at(&created, &["seeded"]);
    assert_eq!(text(seeded, "environment"), "blank");
    assert_eq!(text(seeded, "environment_source"), "option");
    let store = text(at(&created, &["project"]), "path");
    let defined = f.raw(&["definitions", "--store", &store]);
    assert_eq!(defined.0, 0, "{:?}", defined.1);
    let available: Vec<String> = list(&defined.1, "environment_profiles")
        .iter()
        .map(|p| text(p, "designation"))
        .collect();
    assert!(
        available.contains(&"I-300x150".to_owned()),
        "the blank environment answers, not acme: {available:?}"
    );
    assert!(!available.iter().any(|d| d == "AC-BEAM"));

    // With both the actor and the environment on the command line the settings are never read:
    // a damaged file does not block this creation.
    f.damage_settings();
    let created = f.ok(&[
        "create-project",
        "--parent",
        &models,
        "--name",
        "Independent",
        "--environment",
        "blank",
        "--actor-id",
        ACTOR,
    ]);
    assert_eq!(text(&created, "actor_id"), ACTOR);
    assert_eq!(text(at(&created, &["project"]), "state"), "ready");
}

#[test]
fn a_settings_environment_that_is_not_installed_refuses_without_a_folder() {
    let f = Fixture::new();
    f.set_settings_environment("ghost");
    assert_eq!(
        f.code(&[
            "create-project",
            "--parent",
            &f.path("models"),
            "--name",
            "Nothing"
        ]),
        "NOT_FOUND"
    );
    assert!(
        !f.shared.root.join("models/Nothing").exists(),
        "a refused creation leaves no folder"
    );
}

#[test]
fn a_settings_environment_with_a_missing_library_keeps_the_accurate_refusal() {
    let f = Fixture::new();
    f.install_environment("acme", 300.0);
    // The environment declares the library, but the library file is gone: this not-found is
    // about the file, not about the choice, and keeps its own text.
    fs::remove_file(f.shared.root.join("config/environments/acme/acme.usda")).unwrap();
    let models = f.path("models");
    let (code, body) = f.with_config(&["create-project", "--parent", &models, "--name", "Hollow"]);
    assert_eq!(code, 2, "{body:?}");
    assert_eq!(text(&body, "code"), "NOT_FOUND");
    assert_eq!(
        text(&body, "message"),
        "the library file could not be opened"
    );
    assert!(!f.shared.root.join("models/Hollow").exists());
}

#[test]
fn without_any_environment_choice_the_project_is_plain() {
    let f = Fixture::new();
    let config = f.shared.root.join("config");
    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("settings.json"),
        br#"{"environment":null,"models_dir":null,"profile":null,"recent":[],"skip_launcher":false,"version":1}"#,
    )
    .unwrap();
    let (code, created) = f.create("Plain");
    assert_eq!(code, 0, "{created:?}");
    assert!(
        created.get("seeded").is_err(),
        "no choice, no seeding: {created:?}"
    );
    let store = text(at(&created, &["project"]), "path");
    assert!(!Path::new(&store).join("environment").exists());
}

#[test]
fn a_damaged_settings_file_refuses_a_creation_that_needs_the_active_environment() {
    let f = Fixture::new();
    f.damage_settings();
    let models = f.path("models");
    assert_eq!(
        f.code(&["create-project", "--parent", &models, "--name", "Blocked"]),
        "INTEGRITY_ERROR"
    );
    assert!(!f.shared.root.join("models/Blocked").exists());
    // Naming both the actor and the environment keeps the creation independent of the settings
    // file: nothing needs to be read from it.
    let created = f.ok(&[
        "create-project",
        "--parent",
        &models,
        "--name",
        "Independent",
        "--environment",
        "blank",
        "--actor-id",
        ACTOR,
    ]);
    assert_eq!(text(&created, "actor_id"), ACTOR);
    assert_eq!(text(at(&created, &["project"]), "state"), "ready");
}

#[test]
fn an_environment_update_leaves_the_copies_projects_carry_alone() {
    let f = Fixture::new();
    f.install_environment("acme", 300.0);
    let (code, first) = f.create("Pinned");
    assert_eq!(code, 0, "{first:?}");
    let store = text(at(&first, &["project"]), "path");

    // The environment in the application data moves on: same id, new geometry. The next
    // creation takes the update.
    f.install_environment("acme", 400.0);
    let (code, fresh) = f.create("Fresh");
    assert_eq!(code, 0, "{fresh:?}");
    let seeded = at(&fresh, &["seeded"]);
    assert_eq!(text(seeded, "environment"), "acme");
    assert_eq!(text(seeded, "environment_source"), "settings");
    let fresh_store = text(at(&fresh, &["project"]), "path");
    let fresh_kg = f.beam_mass(&fresh_store);
    assert!(
        (fresh_kg - expected_mass(400.0)).abs() < 0.001,
        "{fresh_kg} kg, expected the updated catalogue area"
    );

    // The project created before the update still answers from its own copy: the member takes
    // the geometry the project was created with, not the one the application data now holds.
    let kg = f.beam_mass(&store);
    assert!(
        (kg - expected_mass(300.0)).abs() < 0.001,
        "{kg} kg, the pinned copy answers"
    );
    let old = fs::read_to_string(Path::new(&store).join("environment/acme.usda")).unwrap();
    let new = fs::read_to_string(Path::new(&fresh_store).join("environment/acme.usda")).unwrap();
    assert_ne!(old, new, "the update reached the new project's copy only");
}
