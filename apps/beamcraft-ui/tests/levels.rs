// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic levels; no commercial model or catalogue data.

//! The levels form runs the CLI's own commands: one gesture is one revision and one undo unit.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use beamcraft_json::{Json, Limits};
use beamcraft_ui::levels::{self, LevelForm, NewLevelsForm};
use beamcraft_ui::stats::Stats;
use beamcraft_ui::tools::LevelChoice;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
const ACTOR: &str = "0199e7a0-0000-7000-8000-0000000000a1";

struct Fixture {
    root: PathBuf,
    cli: PathBuf,
    workspace: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let cli = Path::new(env!("CARGO_BIN_EXE_beamcraft-ui"))
            .parent()
            .unwrap()
            .join(format!("beamcraft{}", std::env::consts::EXE_SUFFIX));
        assert!(
            cli.is_file(),
            "build beamcraft-cli before running level UI acceptance"
        );
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("level-ui-tests")
            .join(format!(
                "{}-{}",
                crate::support::boot_nonce(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        std::fs::create_dir_all(&root).unwrap();
        let workspace = root.join("draft");
        let f = Self {
            root,
            cli,
            workspace,
        };
        f.ok(&[
            "create-project",
            "--parent",
            f.root.to_str().unwrap(),
            "--name",
            "ui",
            "--actor-id",
            ACTOR,
            "--config-dir",
            f.root.join("config").to_str().unwrap(),
        ]);
        f.ok(&[
            "create-workspace",
            "--store",
            f.root.join("ui").to_str().unwrap(),
            "--workspace-dir",
            f.workspace.to_str().unwrap(),
        ]);
        f
    }

    fn ok(&self, args: &[&str]) -> Json {
        let out = Command::new(&self.cli).args(args).output().unwrap();
        let json = beamcraft_json::decode(&out.stdout, &Limits::DEFAULT).unwrap();
        assert_eq!(out.status.code(), Some(0), "{args:?}: {json:?}");
        json
    }

    fn head(&self) -> Json {
        self.ok(&["open-revision", "--store", self.workspace.to_str().unwrap()])
    }

    fn history(&self, command: &str) {
        self.ok(&[command, "--workspace-dir", self.workspace.to_str().unwrap()]);
    }

    fn levels(&self) -> Vec<LevelChoice> {
        let reply = self.ok(&["levels", "--store", self.workspace.to_str().unwrap()]);
        reply
            .get("levels")
            .unwrap()
            .array()
            .unwrap()
            .iter()
            .map(|level| LevelChoice {
                id: level.get("id").unwrap().string().unwrap().to_owned(),
                name: level.get("name").unwrap().string().unwrap().to_owned(),
                elevation_mm: level.get("elevation_mm").unwrap().float().unwrap(),
                elements: level.get("elements").unwrap().array().unwrap().len(),
            })
            .collect()
    }

    fn commit(&self, args: &[String]) -> Result<usize, beamcraft_ui::tools::ToolError> {
        levels::commit(&self.cli, args, &Stats::default())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn form(name: &str, elevation: &str) -> LevelForm {
    LevelForm {
        name: name.into(),
        elevation: elevation.into(),
        move_elements: false,
    }
}

#[test]
fn a_level_is_created_edited_undone_and_deleted_through_the_form() {
    let f = Fixture::new();
    let store = f.workspace.clone();
    let one = NewLevelsForm {
        elevations: "3000".into(),
        names: String::new(),
    };
    f.commit(&one.prepare(&store, Some(ACTOR)).unwrap())
        .unwrap();
    let created = f.levels().remove(0);
    assert_eq!(
        (created.name.as_str(), created.elevation_mm),
        ("Level 1", 3000.0)
    );

    f.commit(
        &form("Storey 1", "3300.5")
            .prepare_update(&created, &store, Some(ACTOR))
            .unwrap(),
    )
    .unwrap();
    let edited = f.levels().remove(0);
    assert_eq!(
        (edited.name.as_str(), edited.elevation_mm),
        ("Storey 1", 3300.5)
    );
    assert_eq!(edited.id, created.id);

    // The whole edit is one undo unit, although it wrote the name and the elevation.
    f.history("undo-workspace");
    assert_eq!(f.levels(), std::slice::from_ref(&created));
    f.history("redo-workspace");
    assert_eq!(f.levels(), std::slice::from_ref(&edited));

    let before = f.head();
    assert_ne!(
        f.commit(
            &form("Storey 1", "1e12")
                .prepare_update(&edited, &store, Some(ACTOR))
                .unwrap(),
        )
        .unwrap_err()
        .code,
        ""
    );
    assert_eq!(f.head(), before, "a refusal writes nothing");

    f.commit(&levels::prepare_delete(&edited, &store, Some(ACTOR)).unwrap())
        .unwrap();
    assert!(f.levels().is_empty());
    f.history("undo-workspace");
    assert_eq!(f.levels(), [edited]);
}

#[test]
fn a_spaced_batch_of_levels_is_one_revision_and_one_undo_unit() {
    let f = Fixture::new();
    let store = f.workspace.clone();
    let batch = NewLevelsForm {
        elevations: "-1000 1000 6*3500".into(),
        names: "Basement, Ground".into(),
    };
    let preview = batch.preview(&f.levels()).unwrap();
    f.commit(&batch.prepare(&store, Some(ACTOR)).unwrap())
        .unwrap();
    let mut created: Vec<(String, f64)> = f
        .levels()
        .into_iter()
        .map(|level| (level.name, level.elevation_mm))
        .collect();
    created.sort_by(|a, b| a.1.total_cmp(&b.1));
    assert_eq!(created, preview, "the preview is what the engine created");
    assert_eq!(
        created.first().zip(created.get(2)),
        Some((
            &("Basement".to_owned(), -1000.0),
            &("Ground 2".to_owned(), 3500.0)
        ))
    );
    assert_eq!(created.len(), 8);

    f.history("undo-workspace");
    assert!(f.levels().is_empty(), "one undo removes the whole batch");
    f.history("redo-workspace");
    assert_eq!(f.levels().len(), 8);
}

#[test]
fn a_column_on_a_moved_level_follows_it_and_one_undo_restores_both() {
    let f = Fixture::new();
    let store = f.workspace.clone();
    let store_text = store.to_str().unwrap();
    let profile = f.ok(&[
        "define-profile",
        "--store",
        store_text,
        "--width",
        "200",
        "--height",
        "200",
    ]);
    let digest = profile
        .get("definition")
        .unwrap()
        .get("digest")
        .unwrap()
        .string()
        .unwrap()
        .to_owned();
    f.ok(&[
        "create-member",
        "--store",
        store_text,
        "--kind",
        "column",
        "--start",
        "0,0,0",
        "--end",
        "0,0,3000",
        "--profile-digest",
        &digest,
    ]);
    let top = || {
        let inspected = f.ok(&["inspect-elements", "--store", store_text]);
        let member = &inspected.get("members").unwrap().array().unwrap()[0];
        let axis = member.get("axis_points_mm").unwrap().array().unwrap();
        axis[1].array().unwrap()[2].float().unwrap()
    };
    let batch = NewLevelsForm {
        elevations: "3000".into(),
        names: String::new(),
    };
    f.commit(&batch.prepare(&store, Some(ACTOR)).unwrap())
        .unwrap();
    let level = f.levels().remove(0);
    let mut moved = LevelForm::of(&level);
    moved.elevation = "3600".into();
    let followed = f
        .commit(&moved.prepare_update(&level, &store, Some(ACTOR)).unwrap())
        .unwrap();
    assert_eq!(followed, 1);
    assert_eq!(top(), 3600.0);
    assert_eq!(f.levels()[0].elevation_mm, 3600.0);

    f.history("undo-workspace");
    assert_eq!(top(), 3000.0, "one undo restores the column");
    assert_eq!(f.levels()[0].elevation_mm, 3000.0, "and the level");
}
