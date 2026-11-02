// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The start commands end to end through the real `beamcraft` binary: the settings and the author
//! profile, the built-in `blank` environment and a user environment, creating a project that the
//! model commands open, listing the projects of a folder and the recently opened ones. Every run
//! names its own `--config-dir`, so the user's real settings are never read or written.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support;

use support::{at, list, text};

use beamcraft_json::{Json, Limits};
use std::fs;
use std::process::Command;

const ACTOR: &str = "0199e7a0-0000-7000-8000-0000000000a1";
const PROJECT: &str = "0199e7a0-0000-7000-8000-0000000000b1";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("start-tests"),
        };
        fs::create_dir(fixture.shared.root.join("models")).unwrap();
        fixture
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
        let config = self.path("config");
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
}

#[test]
fn unique_project_names_skip_existing_files_and_reserve_free_folders() {
    let f = Fixture::new();
    let parent = f.path("models");
    let create = || {
        f.ok(&[
            "create-project",
            "--parent",
            &parent,
            "--name",
            "New Model",
            "--unique-name",
            "true",
            "--actor-id",
            ACTOR,
        ])
    };
    assert_eq!(text(at(&create(), &["project"]), "name"), "New Model");
    fs::write(
        f.shared.root.join("models/New Model 1"),
        b"keep this existing file",
    )
    .unwrap();
    assert_eq!(text(at(&create(), &["project"]), "name"), "New Model 2");
    assert_eq!(text(at(&create(), &["project"]), "name"), "New Model 3");
    let mut concurrent = std::thread::scope(|scope| {
        let first = scope.spawn(create);
        let second = scope.spawn(create);
        [first.join().unwrap(), second.join().unwrap()]
            .map(|created| text(at(&created, &["project"]), "name"))
    });
    concurrent.sort();
    assert_eq!(concurrent, ["New Model 4", "New Model 5"]);
    assert_eq!(
        fs::read(f.shared.root.join("models/New Model 1")).unwrap(),
        b"keep this existing file"
    );
    assert_eq!(
        f.code(&[
            "create-project",
            "--parent",
            &parent,
            "--name",
            "New Model",
            "--actor-id",
            ACTOR
        ]),
        "DUPLICATE_ID"
    );
}

#[test]
fn a_created_project_opens_lists_and_is_remembered() {
    let f = Fixture::new();
    let models = f.path("models");
    let created = f.ok(&[
        "create-project",
        "--parent",
        &models,
        "--name",
        "Каркас A-F 1-14",
        "--project-id",
        PROJECT,
        "--actor-id",
        ACTOR,
    ]);
    assert_eq!(text(&created, "actor_id"), ACTOR);
    assert_eq!(at(&created, &["recent_recorded"]), &Json::Bool(true));
    let project = at(&created, &["project"]);
    assert_eq!(text(project, "state"), "ready");
    assert_eq!(text(project, "project_id"), PROJECT);
    let store = text(project, "path");
    assert!(store.ends_with("Каркас A-F 1-14"));

    // The model commands open the new, empty project and record its id.
    let inspected = f.raw(&["inspect-elements", "--store", &store]);
    assert_eq!(inspected.0, 0, "{:?}", inspected.1);
    assert_eq!(text(&inspected.1, "project_id_record"), PROJECT);
    let plate = f.raw(&[
        "create-plate",
        "--store",
        &store,
        "--width",
        "100",
        "--length",
        "200",
        "--thickness",
        "10",
        "--actor-id",
        ACTOR,
    ]);
    assert_eq!(plate.0, 0, "{:?}", plate.1);

    // A second project of the same name, a reserved name and a missing parent are refused, and
    // a refused creation leaves no folder behind.
    let args = |name: &'static str, parent: &str| {
        f.code(&["create-project", "--parent", parent, "--name", name])
    };
    assert_eq!(args("Каркас A-F 1-14", &models), "DUPLICATE_ID");
    assert_eq!(args("CON", &models), "INVALID_ARGUMENT");
    assert_eq!(args("a/b", &models), "INVALID_ARGUMENT");
    assert_eq!(args("x", &f.path("nowhere")), "NOT_FOUND");
    fs::create_dir(f.shared.root.join("models/plain folder")).unwrap();

    let listed = f.raw(&["list-project", "--dir", &models]).1;
    let projects = at(&listed, &["projects"]).array().unwrap();
    assert_eq!(
        projects.len(),
        1,
        "a folder that is not a store is left out"
    );
    let modified = at(&projects[0], &["modified_ms"]).unsigned().unwrap();
    let created_ms = at(&projects[0], &["created_ms"]).unsigned().unwrap();
    assert!(modified >= created_ms);
}

#[test]
fn a_project_created_from_the_blank_environment_places_members_at_once() {
    let f = Fixture::new();
    let models = f.path("models");
    let created = f.ok(&[
        "create-project",
        "--parent",
        &models,
        "--name",
        "Seeded",
        "--environment",
        "blank",
        "--project-id",
        PROJECT,
    ]);
    let seeded = at(&created, &["seeded"]);
    assert_eq!(text(seeded, "environment"), "blank");
    assert_eq!(at(seeded, &["materials"]).unsigned().unwrap(), 2);
    let store = text(at(&created, &["project"]), "path");

    // The project carries its environment, so no command names one: the model starts with the two
    // default materials and lists the environment's profiles as available.
    assert!(
        f.shared
            .root
            .join("models/Seeded/environment/environment.usda")
            .is_file()
    );
    let defined = f.raw(&["definitions", "--store", &store]);
    assert_eq!(defined.0, 0, "{:?}", defined.1);
    assert_eq!(at(&defined.1, &["definition_count"]).unsigned().unwrap(), 2);
    let available = list(&defined.1, "environment_profiles");
    assert!(
        available.len() >= 15,
        "{} environment profiles",
        available.len()
    );
    let place = |kind: &str, end: &str, profile: &str, material: &str| {
        let (code, body) = f.raw(&[
            "create-member",
            "--store",
            &store,
            "--kind",
            kind,
            "--start",
            "0,0,0",
            "--end",
            end,
            "--profile",
            profile,
            "--material",
            material,
        ]);
        assert_eq!(code, 0, "{profile} {material}: {body:?}");
        body
    };
    let beam = place("beam", "6000,0,0", "I-300x150", "Steel");
    // The profile was defined by its first use and is no longer only available.
    let after = f.raw(&["definitions", "--store", &store]).1;
    assert_eq!(at(&after, &["definition_count"]).unsigned().unwrap(), 3);
    assert_eq!(
        list(&after, "environment_profiles").len(),
        available.len() - 1
    );
    let mass = at(&beam, &["member", "quantities", "mass"]);
    assert_eq!(text(mass, "status"), "current");
    assert!(at(mass, &["kg"]).float().unwrap() > 100.0);

    // Steel_Undefined is the massless material of auxiliary parts: a known zero, not unknown.
    let column = place("column", "0,0,4000", "H-200x200", "Steel_Undefined");
    let mass = at(&column, &["member", "quantities", "mass"]);
    assert_eq!(text(mass, "status"), "current");
    assert_eq!(at(mass, &["kg"]).float().unwrap(), 0.0);
}

#[test]
fn an_unknown_environment_creates_no_project() {
    let f = Fixture::new();
    let models = f.path("models");
    let code = f.code(&[
        "create-project",
        "--parent",
        &models,
        "--name",
        "Nothing",
        "--environment",
        "no-such-environment",
    ]);
    assert_eq!(code, "NOT_FOUND");
    assert!(!f.shared.root.join("models/Nothing").exists());
}

#[test]
fn a_placed_member_changes_its_profile_by_name_and_a_plate_refuses() {
    let f = Fixture::new();
    let models = f.path("models");
    let created = f.ok(&[
        "create-project",
        "--parent",
        &models,
        "--name",
        "Swap",
        "--environment",
        "blank",
    ]);
    let store = text(at(&created, &["project"]), "path");
    let run = |args: &[&str]| f.raw(args);
    let beam = run(&[
        "create-member",
        "--store",
        &store,
        "--start",
        "0,0,0",
        "--end",
        "6000,0,0",
        "--profile",
        "I-300x150",
        "--material",
        "Steel",
    ]);
    assert_eq!(beam.0, 0, "{:?}", beam.1);
    let id = text(at(&beam.1, &["member"]), "element_id");
    let before = at(&beam.1, &["member", "quantities", "mass", "kg"])
        .float()
        .unwrap();
    let first_profile = text(at(&beam.1, &["member"]), "profile_digest");

    // A name that the model does not define yet comes from the project's environment.
    let swap = |extra: &[&str]| {
        let mut args = vec!["update-element", "--store", &store, "--id", &id];
        args.extend_from_slice(extra);
        run(&args)
    };
    let changed = swap(&["--profile", "H-300x300", "--op-id", OP]);
    assert_eq!(changed.0, 0, "{:?}", changed.1);
    assert_eq!(at(&changed.1, &["effect", "profile"]), &Json::Bool(true));
    let second_profile = text(at(&changed.1, &["element"]), "profile_digest");
    assert_ne!(first_profile, second_profile);
    let shown = f.raw(&["inspect-elements", "--store", &store]).1;
    assert!(format!("{shown:?}").contains("H-300x300"));
    let after = at(&changed.1, &["element", "quantities", "mass", "kg"])
        .float()
        .unwrap();
    assert!(after > before * 2.0, "{before} kg became {after} kg");

    // The same operation again replays; the same profile under a new operation changes nothing.
    let replay = swap(&["--profile", "H-300x300", "--op-id", OP]);
    assert_eq!(text(&replay.1, "outcome"), "element_update_replayed");
    let nothing = swap(&["--profile", "H-300x300"]);
    assert_eq!(nothing.0, 2);
    assert_eq!(text(&nothing.1, "code"), "INVALID_ARGUMENT");
    assert_eq!(
        swap(&["--profile", "NO-SUCH"]).1.get("code").unwrap(),
        &Json::text("NOT_FOUND")
    );

    // A plate has no profile.
    let plate = run(&[
        "create-plate",
        "--store",
        &store,
        "--width",
        "100",
        "--length",
        "200",
        "--thickness",
        "10",
    ]);
    assert_eq!(plate.0, 0, "{:?}", plate.1);
    let plate_id = text(at(&plate.1, &["plate"]), "element_id");
    let refused = run(&[
        "update-element",
        "--store",
        &store,
        "--id",
        &plate_id,
        "--profile",
        "H-300x300",
    ]);
    assert_eq!(refused.0, 2);
    assert_eq!(text(&refused.1, "code"), "UNSUPPORTED_PROPERTY");
}

const OP: &str = "0199e7a0-0000-7000-8000-0000000000c1";
