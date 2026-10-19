// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates and members, MPL-2.0.

//! V06 end to end: the properties panel's pick and element lookup, the typed update patch and
//! the publication panel's commands, driven against the real `beamcraft` binary, behave exactly
//! like running the CLI by hand (same properties, same saved result, same workspace state).
//! Without the CLI binary next to this package's binary the tests are skipped and say so
//! (`cargo test --workspace` builds it).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use beamcraft_protocol::{
    json::{self, Json},
    message,
};
use beamcraft_ui::edit::Ray;
use beamcraft_ui::props::{self, MaterialEdit, UpdateForm};
use beamcraft_ui::{command, source};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn cli() -> Option<PathBuf> {
    let ui = Path::new(env!("CARGO_BIN_EXE_beamcraft-ui"));
    let cli = ui
        .parent()?
        .join(format!("beamcraft{}", std::env::consts::EXE_SUFFIX));
    if cli.is_file() {
        Some(cli)
    } else {
        eprintln!("SKIP properties test: build beamcraft-cli first (cargo test --workspace does)");
        None
    }
}

struct Stores {
    root: PathBuf,
}

impl Stores {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("v06-tests")
            .join(format!(
                "run-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("ui")).unwrap();
        std::fs::create_dir_all(root.join("hand")).unwrap();
        Self { root }
    }
    fn ui(&self) -> PathBuf {
        self.root.join("ui")
    }
    fn hand(&self) -> PathBuf {
        self.root.join("hand")
    }
}

impl Drop for Stores {
    fn drop(&mut self) {
        assert!(self.root.is_absolute() && self.root.file_name().is_some());
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Runs the CLI by hand with literal arguments; returns the exit status and the JSON reply.
fn by_hand(cli: &Path, args: &[&str]) -> (i32, Json) {
    let output = Command::new(cli).args(args).output().unwrap();
    let body = json::decode(&output.stdout, &message::read_limits()).unwrap();
    (output.status.code().unwrap(), body)
}

fn ok(cli: &Path, args: &[&str]) -> Json {
    let (code, body) = by_hand(cli, args);
    assert_eq!(code, 0, "{args:?}: {body:?}");
    body
}

fn at<'a>(json: &'a Json, path: &[&str]) -> &'a Json {
    path.iter().fold(json, |j, k| j.get(k).unwrap())
}

fn text(json: &Json, path: &[&str]) -> String {
    at(json, path).string().unwrap().to_owned()
}

fn floats(json: &Json) -> [f64; 3] {
    let values = json.array().unwrap();
    [
        values[0].float().unwrap(),
        values[1].float().unwrap(),
        values[2].float().unwrap(),
    ]
}

/// A store with one named plate that pins one named material, as both compared stores start.
fn plate_with_material(cli: &Path, store: &Path, name: &str) -> String {
    let root = store.to_str().unwrap();
    let material = ok(
        cli,
        &[
            "define-material",
            "--store",
            root,
            "--density",
            "7850",
            "--grade",
            "S355",
            "--name",
            "Steel S355",
        ],
    );
    let created = ok(
        cli,
        &[
            "create-plate",
            "--store",
            root,
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "10",
            "--name",
            name,
            "--material-digest",
            &text(&material, &["definition", "digest"]),
        ],
    );
    text(&created, &["plate", "element_id"])
}

#[test]
fn pick_and_lookup_report_what_the_cli_reports() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    let store = stores.ui();
    let id = plate_with_material(&cli, &store, "P1");

    // The properties of the element match the CLI's own report, statuses included.
    let found = command::properties(&cli, &store, &id).unwrap().unwrap();
    assert_eq!(found.element_id, id);
    assert_eq!(found.kind, "plate");
    assert_eq!(found.name, "P1");
    assert_eq!(found.dimension("width"), Some(1000.0));
    assert_eq!(found.dimension("length"), Some(500.0));
    assert_eq!(found.dimension("thickness"), Some(10.0));
    assert_eq!(found.volume_mm3, Some(5_000_000.0));
    assert_eq!(found.volume_status, "current");
    // 5 m3 e-3 x 7850 kg/m3.
    assert!((found.mass_kg.unwrap() - 39.25).abs() < 1e-9);
    assert_eq!(found.mass_status, "current");
    assert_eq!(
        found.material.map(|m| m.name).unwrap(),
        Some("Steel S355".to_owned())
    );
    assert_eq!(found.profile, None);

    // A pick through the plate's centre from above hits it; a ray into the sky does not. The
    // window's own path is `select_at` (the editor's light report); the panel's full properties
    // are the same `plate-inspect` read through `properties`.
    let reply = ok(&cli, &["plate-inspect", "--store", store.to_str().unwrap()]);
    let element = &at(&reply, &["snapshot", "elements"]).array().unwrap()[0];
    let min = floats(at(element, &["world_min_mm"]));
    let max = floats(at(element, &["world_max_mm"]));
    let centre = [
        (min[0] + max[0]) / 2.0,
        (min[1] + max[1]) / 2.0,
        (min[2] + max[2]) / 2.0,
    ];
    let ray = |origin: [f64; 3], direction: [f64; 3]| Ray { origin, direction };
    let selected = command::select_at(
        &cli,
        &store,
        &ray([centre[0], centre[1], max[2] + 1000.0], [0.0, 0.0, -1.0]),
    )
    .unwrap()
    .unwrap();
    assert_eq!(selected.id, id);
    assert_eq!(selected.info.unwrap().name, "P1");
    let missed = command::select_at(
        &cli,
        &store,
        &ray([centre[0], centre[1], max[2] + 1000.0], [0.0, 0.0, 1.0]),
    )
    .unwrap();
    assert!(missed.is_none());

    // A store without a revision refuses the pick with the CLI's own code.
    let empty = stores.hand();
    let error = command::select_at(&cli, &empty, &ray([0.0; 3], [0.0, 0.0, -1.0])).unwrap_err();
    assert_eq!(error.code, "NOT_FOUND", "{error}");

    // A missing element is no error: the panel says it is gone.
    assert_eq!(command::properties(&cli, &store, "gone").unwrap(), None);
}

#[test]
fn an_update_through_the_form_equals_the_same_update_by_hand() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    let (ui, hand) = (stores.ui(), stores.hand());
    let ui_id = plate_with_material(&cli, &ui, "P1");
    let hand_id = plate_with_material(&cli, &hand, "P1");

    // The panel's form stages the exact argument list and the run changes the element.
    let form = UpdateForm {
        name: "P2".into(),
        material: MaterialEdit::Keep,
        thickness: "20".into(),
        translation: ["10".into(), "0".into(), "5".into()],
    };
    let args = form.prepare(&ui, &ui_id).unwrap();
    assert_eq!(args[0], "update-element");
    let change = command::run_change(&cli, &args).unwrap();
    assert_eq!(change.outcome, "element_updated");
    assert_eq!(change.element_id.as_deref(), Some(ui_id.as_str()));

    // The same command by hand, in a second store.
    let (code, reply) = by_hand(
        &cli,
        &[
            "update-element",
            "--store",
            hand.to_str().unwrap(),
            "--id",
            &hand_id,
            "--name",
            "P2",
            "--thickness",
            "20",
            "--translation",
            "10,0,5",
        ],
    );
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, &["outcome"]), "element_updated");

    // The saved results agree where they must (ids are generated per store).
    let lookup = |store: &Path, id: &str| {
        let reply = ok(&cli, &["plate-inspect", "--store", store.to_str().unwrap()]);
        let plates = at(&reply, &["plates"]).array().unwrap();
        plates
            .iter()
            .find(|p| text(p, &["element_id"]) == id)
            .unwrap()
            .clone()
    };
    let (a, b) = (lookup(&ui, &ui_id), lookup(&hand, &hand_id));
    for path in [
        &["name"][..],
        &["dimensions_mm", "thickness"],
        &["quantities", "volume", "mm3"],
        &["placement", "translation_mm"],
        &["mesh_digest"],
    ] {
        assert_eq!(at(&a, path), at(&b, path), "{path:?}");
    }
    assert_eq!(
        at(&a, &["quantities", "volume", "mm3"]).float().unwrap(),
        10_000_000.0
    );

    // The properties refresh shows the new values, including the stale-aware mass.
    let found = command::properties(&cli, &ui, &ui_id).unwrap().unwrap();
    assert_eq!(found.name, "P2");
    assert_eq!(found.dimension("thickness"), Some(20.0));
    assert_eq!(found.translation_mm, [10.0, 0.0, 5.0]);

    // An engineering refusal arrives with the engine's code and changes nothing.
    let bad = UpdateForm {
        thickness: "-5".into(),
        ..UpdateForm::default()
    };
    let args = bad.prepare(&ui, &ui_id).unwrap();
    let error = command::run_change(&cli, &args).unwrap_err();
    assert_eq!(error.code, "INVALID_ARGUMENT", "{error}");
    let unchanged = command::properties(&cli, &ui, &ui_id).unwrap().unwrap();
    assert_eq!(unchanged.dimension("thickness"), Some(20.0));

    // The staging machine refuses to run a stale preview: editing after it disables the commit.
    let mut props = props::Props::default();
    props.form.name = "P9".into();
    props.preview(&ui, &ui_id).unwrap();
    props.form.name = "P8".into();
    assert!(props.begin_commit(&ui, &ui_id).is_none());
}

#[test]
fn the_publication_panel_follows_the_real_draft() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    let (published, workspace) = (stores.ui(), stores.root.join("ws"));
    let id = plate_with_material(&cli, &published, "P1");
    let published_str = published.to_str().unwrap();
    let workspace_str = workspace.to_str().unwrap();

    ok(
        &cli,
        &[
            "workspace-create",
            "--store",
            published_str,
            "--workspace-dir",
            workspace_str,
        ],
    );
    let panel = |cli: &Path| {
        let state = command::workspace_state(cli, &published, &workspace).unwrap();
        let (undo_units, redo_units) = command::workspace_history(cli, &workspace).unwrap();
        props::WorkspacePanel {
            state,
            undo_units,
            redo_units,
        }
    };
    // A fresh fork: nothing unsent, nothing to undo or redo. The fork is a new draft revision
    // holding the published content, so its log starts over at sequence 1.
    let start = panel(&cli);
    assert!(start.state.base_is_current);
    assert_eq!(start.state.unsent_revisions, 0);
    assert_eq!(start.undo_units, 0);
    assert_eq!(start.redo_units, 0);
    assert_eq!(start.state.draft_head.1, 1);

    // The draft is edited through the panel's own mapping.
    let form = UpdateForm {
        name: "P1b".into(),
        ..UpdateForm::default()
    };
    let change = command::run_change(&cli, &form.prepare(&workspace, &id).unwrap()).unwrap();
    assert_eq!(change.outcome, "element_updated");
    let edited = panel(&cli);
    assert_eq!(edited.state.unsent_revisions, 1);
    assert!(edited.state.base_is_current);
    assert_eq!(edited.undo_units, 1);
    assert_eq!(edited.redo_units, 0);
    assert_eq!(
        command::properties(&cli, &workspace, &id)
            .unwrap()
            .unwrap()
            .name,
        "P1b"
    );

    // Publish is the draft head, fast-forward only; afterwards nothing is unsent and the heads
    // are the same revision.
    let change = command::run_change(
        &cli,
        &props::workspace_args("workspace-publish", &published, &workspace),
    )
    .unwrap();
    assert_eq!(change.outcome, "workspace_published");
    let after = panel(&cli);
    assert_eq!(after.state.unsent_revisions, 0);
    assert_eq!(after.state.draft_head.0, after.state.published_head.0);
    assert!(after.state.base_is_current);

    // A second publish of the unchanged draft is the same request: it replays, it does not
    // write a new revision.
    let change = command::run_change(
        &cli,
        &props::workspace_args("workspace-publish", &published, &workspace),
    )
    .unwrap();
    assert_eq!(change.outcome, "workspace_publish_replayed");
    assert_eq!(change.sequence, Some(after.state.published_head.1));
    assert_eq!(panel(&cli).state.unsent_revisions, 0);

    // Undo restores the published state in the draft; redo brings the edit back.
    let change =
        command::run_change(&cli, &props::draft_args("workspace-undo", &workspace)).unwrap();
    assert_eq!(change.outcome, "workspace_undone");
    let undone = panel(&cli);
    assert_eq!(undone.undo_units, 0);
    assert_eq!(undone.redo_units, 1);
    assert_eq!(
        command::properties(&cli, &workspace, &id)
            .unwrap()
            .unwrap()
            .name,
        "P1"
    );
    let change =
        command::run_change(&cli, &props::draft_args("workspace-redo", &workspace)).unwrap();
    assert_eq!(change.outcome, "workspace_redone");
    let redone = panel(&cli);
    assert_eq!(redone.undo_units, 1);
    assert_eq!(redone.redo_units, 0);
    assert_eq!(
        command::properties(&cli, &workspace, &id)
            .unwrap()
            .unwrap()
            .name,
        "P1b"
    );

    // The published store keeps what was published (material, plate, the published draft head);
    // undo and redo never touched it.
    let head = ok(&cli, &["plate-inspect", "--store", published_str]);
    assert_eq!(text(&head, &["revision", "sequence"]), "3");

    // A refresh of the whole window reads the draft and the panel without a window or GPU.
    let reloaded = source::reload(
        &cli,
        &workspace,
        Some(id.as_str()),
        Some((published.as_path(), workspace.as_path())),
    );
    let panel = reloaded.workspace.unwrap().unwrap();
    assert_eq!(panel, redone);
    assert_eq!(
        reloaded.properties.unwrap().unwrap().unwrap().element_id,
        id
    );
}
