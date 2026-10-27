// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The start commands end to end through the real `beamcraft` binary: the settings and the author
//! profile, the built-in `blank` environment and a user environment, creating a project that the
//! model commands open, listing the projects of a folder and the recently opened ones. Every run
//! names its own `--config-dir`, so the user's real settings are never read or written.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_json::{Json, Limits};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

const ACTOR: &str = "0199e7a0-0000-7000-8000-0000000000a1";
const PROJECT: &str = "0199e7a0-0000-7000-8000-0000000000b1";

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("start-tests");
        fs::create_dir_all(&base).unwrap();
        let root = base.join(format!(
            "e2e-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("models")).unwrap();
        Self { root }
    }
    fn path(&self, name: &str) -> String {
        self.root.join(name).to_str().unwrap().to_owned()
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

impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.root.is_absolute() && self.root.file_name().is_some());
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn text(json: &Json, key: &str) -> String {
    json.get(key).unwrap().string().unwrap().to_owned()
}

#[test]
fn default_project_names_skip_existing_files_and_reserve_free_folders() {
    let f = Fixture::new();
    let parent = f.path("models");
    let suggest = || {
        f.raw(&[
            "suggest-project-name",
            "--parent",
            &parent,
            "--name",
            "New Model",
        ])
    };
    assert_eq!(text(&suggest().1, "name"), "New Model");
    assert!(!f.root.join("models/New Model").exists());
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
    assert_eq!(text(&suggest().1, "name"), "New Model 1");
    fs::write(
        f.root.join("models/New Model 1"),
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
        fs::read(f.root.join("models/New Model 1")).unwrap(),
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
    assert_eq!(
        f.raw(&[
            "suggest-project-name",
            "--parent",
            &parent,
            "--name",
            "../bad"
        ])
        .0,
        2
    );
}

#[test]
fn listing_recents_persists_removal_of_missing_models_and_keeps_unreadable_models() {
    use beamcraft_host::settings::Settings;
    let f = Fixture::new();
    let config = f.root.join("config");
    let models = f.path("models");
    let created = f.ok(&[
        "create-project",
        "--parent",
        &models,
        "--name",
        "Ready",
        "--actor-id",
        ACTOR,
    ]);
    let ready = PathBuf::from(text(at(&created, &["project"]), "path"));
    let unreadable = f.root.join("models/unreadable");
    fs::create_dir_all(unreadable.join("revisions")).unwrap();
    fs::write(
        unreadable.join("revisions/corrupt-record"),
        b"not a revision",
    )
    .unwrap();
    let missing = f.root.join("models/removed");
    let no_store = f.root.join("models/removed-store");
    fs::create_dir_all(&no_store).unwrap();
    let mut settings = Settings::load(&config).unwrap();
    for path in [&unreadable, &missing, &no_store] {
        settings.touch_recent(path.clone(), 1).unwrap();
    }
    settings.save(&config).unwrap();
    let listed = f.ok(&["list-recent"]);
    let entries = at(&listed, &["recent"]).array().unwrap();
    assert_eq!(entries.len(), 2, "{listed:?}");
    let saved = Settings::load(&config).unwrap();
    assert_eq!(
        saved
            .recent
            .iter()
            .map(|entry| &entry.path)
            .collect::<Vec<_>>(),
        [&unreadable, &ready]
    );
    assert!(
        !at(&entries[0], &["project", "state"])
            .get("unreadable")
            .unwrap()
            .string()
            .unwrap()
            .is_empty()
    );
    let before = fs::read(config.join("settings.json")).unwrap();
    assert_eq!(listed, f.ok(&["list-recent"]));
    assert_eq!(before, fs::read(config.join("settings.json")).unwrap());
}

fn at<'a>(json: &'a Json, path: &[&str]) -> &'a Json {
    path.iter().fold(json, |j, k| j.get(k).unwrap())
}

#[test]
fn the_profile_keeps_its_author_identity() {
    let f = Fixture::new();
    let empty = f.ok(&["show-settings"]);
    assert_eq!(at(&empty, &["settings", "profile"]), &Json::Null);
    let saved = f.ok(&[
        "set-settings",
        "--name",
        "Юрий",
        "--email",
        "yuri@example.cl",
        "--environment",
        "blank",
        "--skip-launcher",
        "true",
    ]);
    let actor = text(at(&saved, &["settings", "profile"]), "actor_id");
    let changed = f.ok(&["set-settings", "--name", "Yuri", "--email", "y@example.com"]);
    let profile = at(&changed, &["settings", "profile"]);
    assert_eq!(text(profile, "actor_id"), actor);
    assert_eq!(text(profile, "email"), "y@example.com");
    assert_eq!(text(at(&changed, &["settings"]), "environment"), "blank");
    assert_eq!(
        f.code(&["set-settings", "--name", "A", "--email", "not-an-address"]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(f.code(&["set-settings", "--name", "A"]), "INVALID_ARGUMENT");
    // A damaged settings file is refused, never replaced silently.
    fs::write(f.root.join("config/settings.json"), b"{").unwrap();
    assert_eq!(f.code(&["show-settings"]), "INTEGRITY_ERROR");
}

#[test]
fn the_blank_environment_is_built_in_and_a_user_one_is_listed_after_it() {
    let f = Fixture::new();
    let user = f.root.join("config/environments");
    fs::create_dir_all(user.join("mine")).unwrap();
    fs::create_dir_all(user.join("again")).unwrap();
    fs::create_dir_all(user.join("broken")).unwrap();
    let env = |id: &str| {
        format!(
            "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {{\n        string \"bim:environment:id\" = \"{id}\"\n        string[] \"bim:environment:libraries\" = [\"profiles.usda\"]\n    }}\n)\n"
        )
    };
    fs::write(user.join("mine/environment.usda"), env("mine")).unwrap();
    fs::write(user.join("again/environment.usda"), env("blank")).unwrap();
    fs::write(user.join("broken/environment.usda"), "#usda 1.0\n").unwrap();
    let listed = f.ok(&["list-environment"]);
    let entries = at(&listed, &["environments"]).array().unwrap();
    let first = &entries[0];
    assert_eq!(
        (text(first, "id"), text(first, "origin")),
        ("blank".into(), "builtin".into())
    );
    assert_eq!(at(first, &["shadowed"]), &Json::Bool(false));
    let user_entries: Vec<&Json> = entries
        .iter()
        .filter(|e| text(e, "origin") == "user")
        .collect();
    assert_eq!(user_entries.len(), 3);
    let shadowed = user_entries
        .iter()
        .find(|e| at(e, &["id"]) == &Json::text("blank"))
        .unwrap();
    assert_eq!(at(shadowed, &["shadowed"]), &Json::Bool(true));
    assert!(
        user_entries
            .iter()
            .any(|e| at(e, &["refused"]) != &Json::Null && at(e, &["id"]) == &Json::Null)
    );
    // The built-in environment is an ordinary A07.3 environment.
    let blank = text(first, "path");
    let library = f.raw(&["list-library", "--environment", &blank]);
    assert_eq!(library.0, 0, "{:?}", library.1);
}

#[test]
fn a_created_project_opens_lists_and_is_remembered() {
    let f = Fixture::new();
    f.ok(&["set-settings", "--name", "Ana", "--email", "ana@example.cl"]);
    let author = text(
        at(&f.ok(&["show-settings"]), &["settings", "profile"]),
        "actor_id",
    );
    let models = f.path("models");
    let created = f.ok(&[
        "create-project",
        "--parent",
        &models,
        "--name",
        "Каркас A-F 1-14",
        "--project-id",
        PROJECT,
    ]);
    assert_eq!(text(&created, "actor_id"), author);
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
    fs::create_dir(f.root.join("models/plain folder")).unwrap();

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

    let recent = f.ok(&["list-recent"]);
    let recent = at(&recent, &["recent"]).array().unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(text(at(&recent[0], &["project"]), "path"), store);
    assert_eq!(
        f.code(&["add-recent", "--store", &f.path("models/plain folder")]),
        "NOT_FOUND"
    );
    assert_eq!(
        text(&f.ok(&["add-recent", "--store", &store]), "outcome"),
        "recent_added"
    );
}
