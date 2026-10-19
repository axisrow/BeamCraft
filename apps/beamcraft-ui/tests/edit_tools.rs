// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates and members, MPL-2.0.

//! V05.2 end to end: selecting an element from the accepted snapshot's rows and moving or
//! deleting it through the tool mapping runs the same `beamcraft update-element` /
//! `delete-element` command a user types, with the same result (same placement, same mesh
//! digest, same volume); the selection follows the accepted revision; a refusal reaches the
//! panel with the engine's code and message and changes nothing. Without the CLI binary next
//! to this package's binary the tests are skipped (`cargo test --workspace` builds it).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use beamcraft_engine::Representation;
use beamcraft_protocol::{
    json::{self, Json},
    message,
};
use beamcraft_ui::scene::Scene;
use beamcraft_ui::tools::{EditResult, ElementRow, ToolKind, Tools};
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
        eprintln!("SKIP edit-tools test: build beamcraft-cli first (cargo test --workspace does)");
        None
    }
}

struct Stores {
    root: PathBuf,
}

impl Stores {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("v05-edit-tests")
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

fn numbers(json: &Json, path: &[&str]) -> Vec<f64> {
    at(json, path)
        .array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect()
}

/// The only element of a one-element store, inspected by hand.
fn inspected(cli: &Path, store: &Path) -> Json {
    let reply = ok(cli, &["plate-inspect", "--store", store.to_str().unwrap()]);
    let plates = at(&reply, &["plates"]).array().unwrap();
    assert_eq!(plates.len(), 1);
    plates[0].clone()
}

/// The rows the window's Elements list shows: the accepted snapshot's elements.
fn rows(cli: &Path, store: &Path) -> Vec<ElementRow> {
    let snapshot = source::load(cli, store).unwrap();
    snapshot
        .elements()
        .iter()
        .map(|e| ElementRow {
            id: e.id.to_string(),
            kind: e.kind.as_token().to_owned(),
            placement_mm: e.placement_mm,
        })
        .collect()
}

const UNKNOWN: &str = "01990000-0000-7000-8000-00000000000a";

#[test]
fn a_selected_element_moves_and_is_deleted_through_the_engine_commands() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    let (ui, hand) = (stores.ui(), stores.hand());
    for store in [&ui, &hand] {
        ok(
            &cli,
            &[
                "create-plate",
                "--store",
                store.to_str().unwrap(),
                "--width",
                "1000",
                "--length",
                "500",
                "--thickness",
                "10",
                "--name",
                "P1",
            ],
        );
    }

    // The list the panel shows: one plate at the origin, with the id the CLI gave it.
    let listed = rows(&cli, &ui);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].kind, "plate");
    assert_eq!(listed[0].placement_mm, [0.0, 0.0, 0.0]);
    let id = listed[0].id.clone();
    assert_eq!(text(&inspected(&cli, &ui), &["element_id"]), id);

    // Select, then move: the same update-element command a user types.
    let mut tools = Tools::default();
    tools.active = ToolKind::Move;
    tools.select(&listed[0]);
    assert_eq!(tools.move_to, ["0", "0", "0"]);
    tools.move_to = ["500".into(), "0".into(), "250".into()];
    tools.preview(&ui).unwrap();
    let staged = tools.begin_commit(&ui).unwrap();
    assert_eq!(
        staged.args,
        [
            "update-element",
            "--store",
            ui.to_str().unwrap(),
            "--id",
            id.as_str(),
            "--translation",
            "500,0,250"
        ]
    );
    let moved = match command::run_tool(&cli, &staged).unwrap() {
        EditResult::Updated(moved) => moved,
        other => panic!("expected an update reply, got {other:?}"),
    };
    tools.finish();
    assert_eq!(moved.element_id, id);
    assert_eq!(moved.volume_mm3, Some(5_000_000.0));
    assert_eq!(moved.revision.sequence, 2);
    assert!(!moved.replayed);
    assert_eq!(
        EditResult::Updated(moved.clone()).summary(),
        format!(
            "Moved element {id} (P1, volume 0.005 m3). Accepted revision {} (sequence 2).",
            moved.revision.manifest_digest.get(..12).unwrap()
        )
    );

    // The same move by hand, in a second store: the saved results agree, and the mesh the
    // body was built from did not change.
    ok(
        &cli,
        &[
            "update-element",
            "--store",
            hand.to_str().unwrap(),
            "--id",
            &text(&inspected(&cli, &hand), &["element_id"]),
            "--translation",
            "500,0,250",
        ],
    );
    let (a, b) = (inspected(&cli, &ui), inspected(&cli, &hand));
    for path in [
        &["mesh_digest"][..],
        &["quantities", "volume", "mm3"],
        &["placement", "translation_mm"],
    ] {
        assert_eq!(at(&a, path), at(&b, path), "{path:?}");
    }
    assert_eq!(
        numbers(&a, &["placement", "translation_mm"]),
        vec![500.0, 0.0, 250.0]
    );

    // The reload pins the revision the move reported, the listed placement is the new one,
    // and the selection survives it.
    let snapshot = source::load(&cli, &ui).unwrap();
    assert_eq!(
        snapshot.pin().revision().to_string(),
        moved.revision.manifest_digest
    );
    let scene = Scene::from_snapshot(&snapshot, Representation::Full).unwrap();
    assert_eq!(scene.elements, 1);
    tools.retain_selection(|wanted| {
        snapshot
            .elements()
            .iter()
            .any(|e| e.id.to_string() == wanted)
    });
    assert_eq!(
        tools.selection.as_ref().map(|s| s.id.as_str()),
        Some(id.as_str())
    );
    assert_eq!(rows(&cli, &ui)[0].placement_mm, [500.0, 0.0, 250.0]);
    // The staged move (to 500,0,250) is spent: the element is already there, the panel
    // refuses to commit it again without a new preview.
    assert!(!tools.can_commit(&ui));

    // Select, then delete.
    tools.active = ToolKind::Delete;
    tools.preview(&ui).unwrap();
    let staged = tools.begin_commit(&ui).unwrap();
    assert_eq!(
        staged.args,
        [
            "delete-element",
            "--store",
            ui.to_str().unwrap(),
            "--id",
            id.as_str()
        ]
    );
    let deleted = match command::run_tool(&cli, &staged).unwrap() {
        EditResult::Deleted(deleted) => deleted,
        other => panic!("expected a delete reply, got {other:?}"),
    };
    tools.finish();
    assert_eq!(deleted.element_id, id);
    assert_eq!(deleted.kind.as_deref(), Some("plate"));
    assert_eq!(deleted.revision.sequence, 3);
    assert!(!deleted.replayed);
    assert_eq!(
        EditResult::Deleted(deleted.clone()).summary(),
        format!(
            "Deleted plate {id}. Accepted revision {} (sequence 3).",
            deleted.revision.manifest_digest.get(..12).unwrap()
        )
    );
    // The same delete by hand: both stores end without the element.
    ok(
        &cli,
        &[
            "delete-element",
            "--store",
            hand.to_str().unwrap(),
            "--id",
            &text(&inspected(&cli, &hand), &["element_id"]),
        ],
    );
    let snapshot = source::load(&cli, &ui).unwrap();
    assert_eq!(
        Scene::from_snapshot(&snapshot, Representation::Full)
            .unwrap()
            .elements,
        0
    );
    tools.retain_selection(|wanted| {
        snapshot
            .elements()
            .iter()
            .any(|e| e.id.to_string() == wanted)
    });
    assert_eq!(tools.selection, None, "a deleted element is deselected");
    assert_eq!(rows(&cli, &ui).len(), 0);
    assert_eq!(rows(&cli, &hand).len(), 0);
}

#[test]
fn a_refusal_reaches_the_panel_and_publishes_nothing() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    let ui = stores.ui();
    ok(
        &cli,
        &[
            "create-plate",
            "--store",
            ui.to_str().unwrap(),
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "10",
        ],
    );
    let before = source::load(&cli, &ui).unwrap();

    // A move of an element the store does not have (a well-formed id is still an address
    // the engine must resolve).
    let mut tools = Tools::default();
    tools.active = ToolKind::Move;
    tools.select(&ElementRow {
        id: UNKNOWN.into(),
        kind: "plate".into(),
        placement_mm: [0.0; 3],
    });
    tools.move_to = ["10".into(), "0".into(), "0".into()];
    tools.preview(&ui).unwrap();
    let staged = tools.begin_commit(&ui).unwrap();
    let error = command::run_tool(&cli, &staged).unwrap_err();
    tools.finish();
    assert_eq!(error.code, "NOT_FOUND", "{error}");
    let (_, reply) = by_hand(
        &cli,
        &[
            "update-element",
            "--store",
            ui.to_str().unwrap(),
            "--id",
            UNKNOWN,
            "--translation",
            "10,0,0",
        ],
    );
    assert_eq!(error.message, text(&reply, &["message"]));

    // The same for a delete.
    tools.active = ToolKind::Delete;
    tools.preview(&ui).unwrap();
    let staged = tools.begin_commit(&ui).unwrap();
    let error = command::run_tool(&cli, &staged).unwrap_err();
    assert_eq!(error.code, "NOT_FOUND", "{error}");

    // Nothing was published: the same revision, still one element.
    let after = source::load(&cli, &ui).unwrap();
    assert_eq!(
        after.pin().revision().to_string(),
        before.pin().revision().to_string()
    );
    assert_eq!(after.elements().len(), 1);
    assert_eq!(rows(&cli, &ui)[0].placement_mm, [0.0; 3]);
}
