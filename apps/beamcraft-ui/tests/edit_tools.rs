// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, MPL-2.0.

//! V05.2 end to end: selecting by a click, moving, copying, deleting, undo and redo, driven
//! through the edit tools' mapping and reply parsing against the real `beamcraft` binary, give
//! exactly what the CLI gives by hand (same placement, same mesh digest, same world bounds); a
//! click finds the element under it in this process, without starting the CLI; a lookup that is no longer
//! wanted stops its process; a cancelled preview publishes nothing; a refusal
//! reaches the panel with the engine's code and message. The CLI binary must exist next to this
//! package's binary; missing prerequisites fail acceptance.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use beamcraft_engine::Representation;
use beamcraft_engine::query::pick::SelectionFilter;
use beamcraft_json::{Json, Limits};
use beamcraft_ui::camera::Camera;
use beamcraft_ui::edit::{EditKind, Editor, Target, ndc_of, pick_in, pick_point_in, ray_through};
use beamcraft_ui::scene::Scene;
use beamcraft_ui::stats::Stats;
use beamcraft_ui::{command, source};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

const A: &str = "019a7bf0-0000-7000-8000-00000000a001";
const B: &str = "019a7bf0-0000-7000-8000-00000000a002";
const IBEAM: &str = "019a7bf0-0000-7000-8000-00000000a003";
const HOLED: &str = "019a7bf0-0000-7000-8000-00000000a004";

use crate::support;

fn cli() -> Option<PathBuf> {
    support::cli(Path::new(env!("CARGO_BIN_EXE_beamcraft-ui")))
}

struct Stores {
    root: PathBuf,
}

impl Stores {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("v05-2-tests")
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
    fn workspace(&self) -> PathBuf {
        self.root.join("workspace")
    }
}

impl Drop for Stores {
    fn drop(&mut self) {
        assert!(self.root.is_absolute() && self.root.file_name().is_some());
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn by_hand(cli: &Path, args: &[&str]) -> (i32, Json) {
    let output = Command::new(cli).args(args).output().unwrap();
    let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
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

fn s(path: &Path) -> &str {
    path.to_str().unwrap()
}

/// Two plates, A at the origin and B 3 m away, with fixed ids.
fn two_plates(cli: &Path, store: &Path) {
    let store = s(store);
    ok(
        cli,
        &[
            "create-plate",
            "--store",
            store,
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "10",
            "--name",
            "A",
            "--new-id",
            A,
        ],
    );
    ok(
        cli,
        &[
            "create-plate",
            "--store",
            store,
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "10",
            "--name",
            "B",
            "--translation",
            "3000,0,0",
            "--new-id",
            B,
        ],
    );
}

/// What the store holds about the plates: (id, translation, mesh digest, world bounds), by id.
fn state(cli: &Path, store: &Path) -> Vec<(String, String, String, String)> {
    let reply = ok(cli, &["inspect-elements", "--store", s(store)]);
    let plates = at(&reply, &["plates"]).array().unwrap();
    let snapshot = at(&reply, &["snapshot", "elements"]).array().unwrap();
    plates
        .iter()
        .map(|p| {
            let id = text(p, &["element_id"]);
            let bounds = snapshot
                .iter()
                .find(|e| text(e, &["id"]) == id)
                .map(|e| {
                    format!(
                        "{:?}{:?}",
                        at(e, &["world_min_mm"]),
                        at(e, &["world_max_mm"])
                    )
                })
                .unwrap_or_default();
            (
                id,
                format!("{:?}", at(p, &["placement", "translation_mm"])),
                text(p, &["mesh_digest"]),
                bounds,
            )
        })
        .collect()
}

fn head(cli: &Path, store: &Path) -> String {
    text(
        &ok(cli, &["open-revision", "--store", s(store)]),
        &["revision_id"],
    )
}

fn target(store: &Path) -> Target<'_> {
    Target {
        store,
        workspace: None,
    }
}

/// Selects `id` the way the window does after a click: the details from the engine's inspection.
fn select(cli: &Path, store: &Path, editor: &mut Editor, id: &str) {
    editor.select(command::element_info(cli, store, id, &Stats::default()).unwrap());
}

/// Runs a staged edit exactly as the window's Commit does.
fn commit(
    cli: &Path,
    editor: &mut Editor,
    kind: EditKind,
    target: Target<'_>,
) -> Result<beamcraft_ui::edit::Edited, beamcraft_ui::tools::ToolError> {
    editor.preview(kind, target).unwrap();
    let edit = editor.begin_commit(target).unwrap();
    let result = command::commit_edit(cli, &edit, &Stats::default());
    editor.finish();
    result
}

/// V13 acceptance: with the orthographic projection on, the pixel of a plate's top-face centre
/// still names that plate (picking) and the ray meets the plate at the clicked point (snaps).
#[test]
fn an_orthographic_click_picks_and_snaps_like_the_perspective_one() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    two_plates(&cli, &stores.ui());
    let snapshot = source::load(&cli, &stores.ui(), &Stats::default()).unwrap();
    let scene = Scene::from_snapshot(&snapshot, Representation::Full).unwrap();
    let (framing_target, extent) = scene.framing();
    let aspect = 1.6;
    let mut camera = Camera::new(framing_target, extent, aspect).unwrap();
    camera.set_projection(true);
    let view = camera.view(aspect).unwrap();
    let size = [1600.0f32, 1000.0];
    for (id, world) in [(A, [500.0, 250.0, 10.0]), (B, [3500.0, 250.0, 10.0])] {
        let relative = [0, 1, 2].map(|i| (world[i] - scene.origin_mm[i]) as f32);
        let [nx, ny, _] = view.project(relative);
        let pixel = [(nx * 0.5 + 0.5) * size[0], (0.5 - ny * 0.5) * size[1]];
        let ndc = ndc_of(pixel, [0.0, 0.0], size).unwrap();
        let ray = ray_through(&view, ndc, scene.origin_mm).unwrap();
        // Picking: the window's own query finds the plate under the pixel.
        assert_eq!(
            pick_in(&snapshot, &ray, SelectionFilter::ALL)
                .unwrap()
                .as_deref(),
            Some(id)
        );
        // Snapping: a placement click meets the plate at the point the pixel shows.
        let (point, element) = pick_point_in(&snapshot, &ray).unwrap().unwrap();
        assert_eq!(element, id);
        for axis in 0..3 {
            assert!(
                (point[axis] - world[axis]).abs() < 1.0,
                "{id}: {point:?} vs {world:?}"
            );
        }
    }
    // A pixel on empty space still selects nothing.
    let ray = ray_through(&view, [-0.99, 0.99], scene.origin_mm).unwrap();
    assert_eq!(
        pick_in(&snapshot, &ray, SelectionFilter::ALL).unwrap(),
        None
    );
}

#[test]
fn a_click_selects_what_the_engine_picks() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    two_plates(&cli, &stores.ui());
    let snapshot = source::load(&cli, &stores.ui(), &Stats::default()).unwrap();
    let scene = Scene::from_snapshot(&snapshot, Representation::Full).unwrap();
    let (framing_target, extent) = scene.framing();
    let aspect = 1.6;
    let camera = Camera::new(framing_target, extent, aspect).unwrap();
    let view = camera.view(aspect).unwrap();
    // The pixel of the middle of each plate's top face, in a 1600 x 1000 viewport.
    let size = [1600.0f32, 1000.0];
    for (id, world) in [(A, [500.0, 250.0, 10.0]), (B, [3500.0, 250.0, 10.0])] {
        let relative = [0, 1, 2].map(|i| (world[i] - scene.origin_mm[i]) as f32);
        let [nx, ny, _] = view.project(relative);
        let pixel = [(nx * 0.5 + 0.5) * size[0], (0.5 - ny * 0.5) * size[1]];
        let ndc = ndc_of(pixel, [0.0, 0.0], size).unwrap();
        let ray = ray_through(&view, ndc, scene.origin_mm).unwrap();
        // The window picks in its own process on the snapshot it shows.
        let selected = command::select(
            &cli,
            &stores.ui(),
            &snapshot,
            &ray,
            &AtomicBool::new(false),
            &Stats::default(),
            SelectionFilter::ALL,
        )
        .unwrap()
        .unwrap();
        assert_eq!(selected.id, id);
        let info = selected.info.unwrap();
        // The details of the same one-element reply agree with the light report.
        assert_eq!(selected.details.unwrap().info(), info);
        assert_eq!(info.kind, "plate");
        assert_eq!(info.name, if id == A { "A" } else { "B" });
        assert_eq!(
            info.translation_mm,
            if id == A {
                [0.0; 3]
            } else {
                [3000.0, 0.0, 0.0]
            }
        );
    }
    // A pixel on empty space (the top-left corner) selects nothing.
    let ray = ray_through(&view, [-0.99, 0.99], scene.origin_mm).unwrap();
    assert_eq!(
        pick_in(&snapshot, &ray, SelectionFilter::ALL).unwrap(),
        None
    );
    let picked = command::select(
        &cli,
        &stores.ui(),
        &snapshot,
        &ray,
        &AtomicBool::new(false),
        &Stats::default(),
        SelectionFilter::ALL,
    )
    .unwrap();
    assert!(picked.is_none());
    // A click that is not inside the view is not a click.
    assert!(ndc_of([1700.0, 10.0], [0.0, 0.0], size).is_none());
}

#[test]
fn a_click_selects_an_i_section_member_and_a_holed_plate() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    let ui = stores.ui();
    let store = s(&ui);
    // The shapes of the issue #63 reproducer: an invented I-section member and a plate with a
    // round through hole. Neither body has box FeatureIds, so before the engine's mesh pick a
    // click on either cleared the selection.
    let profile = text(
        &ok(
            &cli,
            &[
                "define-profile",
                "--store",
                store,
                "--family",
                "i_shape",
                "--h",
                "200",
                "--b",
                "100",
                "--tw",
                "6",
                "--tf",
                "10",
                "--r",
                "0",
            ],
        ),
        &["definition", "digest"],
    );
    ok(
        &cli,
        &[
            "create-member",
            "--store",
            store,
            "--start",
            "0,0,0",
            "--end",
            "1000,0,0",
            "--translation",
            "0,400,0",
            "--profile-digest",
            &profile,
            "--name",
            "I",
            "--new-id",
            IBEAM,
        ],
    );
    ok(
        &cli,
        &[
            "create-plate",
            "--store",
            store,
            "--width",
            "300",
            "--length",
            "300",
            "--thickness",
            "10",
            "--hole-round",
            "150,150,100",
            "--translation",
            "300,800,0",
            "--name",
            "P",
            "--new-id",
            HOLED,
        ],
    );
    let snapshot = source::load(&cli, &stores.ui(), &Stats::default()).unwrap();
    let scene = Scene::from_snapshot(&snapshot, Representation::Full).unwrap();
    let (framing_target, extent) = scene.framing();
    let aspect = 1.6;
    let camera = Camera::new(framing_target, extent, aspect).unwrap();
    let view = camera.view(aspect).unwrap();
    let size = [1600.0f32, 1000.0];
    // A pixel on the I-beam's top flange (h is world-up, so its surface is at z = 100) and one
    // on solid plate material beside the hole: each click selects the part and loads its
    // properties, and the CLI's own pick answers the same element.
    for (id, world, kind, translation) in [
        (IBEAM, [500.0, 400.0, 100.0], "beam", [0.0, 400.0, 0.0]),
        (HOLED, [350.0, 850.0, 10.0], "plate", [300.0, 800.0, 0.0]),
    ] {
        let relative = [0, 1, 2].map(|i| (world[i] - scene.origin_mm[i]) as f32);
        let [nx, ny, _] = view.project(relative);
        let pixel = [(nx * 0.5 + 0.5) * size[0], (0.5 - ny * 0.5) * size[1]];
        let ndc = ndc_of(pixel, [0.0, 0.0], size).unwrap();
        let ray = ray_through(&view, ndc, scene.origin_mm).unwrap();
        let selected = command::select(
            &cli,
            &stores.ui(),
            &snapshot,
            &ray,
            &AtomicBool::new(false),
            &Stats::default(),
            SelectionFilter::ALL,
        )
        .unwrap()
        .unwrap();
        assert_eq!(selected.id, id);
        let info = selected.info.unwrap();
        assert_eq!(info.kind, kind);
        assert_eq!(info.translation_mm, translation);
    }
}

#[test]
fn a_move_from_the_tool_equals_the_move_by_hand() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    two_plates(&cli, &stores.ui());
    two_plates(&cli, &stores.hand());
    let mut editor = Editor::default();
    select(&cli, &stores.ui(), &mut editor, A);
    assert_eq!(editor.move_to, ["0.0", "0.0", "0.0"]);
    editor.move_to = ["250".into(), "-125.5".into(), "40".into()];
    let ui = stores.ui();
    let done = commit(&cli, &mut editor, EditKind::Move, target(&ui)).unwrap();
    assert_eq!(done.element_id.as_deref(), Some(A));
    ok(
        &cli,
        &[
            "update-element",
            "--store",
            s(&stores.hand()),
            "--id",
            A,
            "--translation",
            "250,-125.5,40",
        ],
    );
    assert_eq!(state(&cli, &stores.ui()), state(&cli, &stores.hand()));
    // The reply names the accepted revision: the head of the store.
    let head_reply = ok(&cli, &["open-revision", "--store", s(&stores.ui())]);
    assert_eq!(
        done.manifest_digest,
        text(&head_reply, &["manifest_digest"])
    );
    assert_eq!(done.sequence, 3);
    // The moved plate is the other one's neighbour now: the selection re-reads the new place.
    select(&cli, &stores.ui(), &mut editor, A);
    assert_eq!(
        editor.selected().unwrap().translation_mm,
        [250.0, -125.5, 40.0]
    );
}

#[test]
fn a_copy_and_a_delete_from_the_tool_equal_the_same_by_hand() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    two_plates(&cli, &stores.ui());
    two_plates(&cli, &stores.hand());
    let ui = stores.ui();
    let mut editor = Editor::default();
    select(&cli, &ui, &mut editor, A);
    editor.copy_offset = ["0".into(), "800".into(), "0".into()];
    let copied = commit(&cli, &mut editor, EditKind::Copy, target(&ui)).unwrap();
    let copy_id = copied.element_id.unwrap();
    assert_ne!(copy_id, A);
    ok(
        &cli,
        &[
            "copy-element",
            "--store",
            s(&stores.hand()),
            "--id",
            A,
            "--offset",
            "0,800,0",
            "--new-id",
            &copy_id,
        ],
    );
    assert_eq!(state(&cli, &ui), state(&cli, &stores.hand()));
    // The copy shares the source's body and sits where the offset put it.
    let all = state(&cli, &ui);
    let source = all.iter().find(|e| e.0 == A).unwrap();
    let copy = all.iter().find(|e| e.0 == copy_id).unwrap();
    assert_eq!(source.2, copy.2);
    assert_ne!(source.1, copy.1);
    // Delete the copy.
    select(&cli, &ui, &mut editor, &copy_id);
    let deleted = commit(&cli, &mut editor, EditKind::Delete, target(&ui)).unwrap();
    assert_eq!(deleted.element_id.as_deref(), Some(copy_id.as_str()));
    ok(
        &cli,
        &[
            "delete-element",
            "--store",
            s(&stores.hand()),
            "--id",
            &copy_id,
        ],
    );
    assert_eq!(state(&cli, &ui), state(&cli, &stores.hand()));
    assert_eq!(state(&cli, &ui).len(), 2);
    // The deleted element is gone from the accepted revision.
    assert_eq!(
        command::element_info(&cli, &ui, &copy_id, &Stats::default())
            .unwrap_err()
            .code,
        "NOT_FOUND"
    );
}

#[test]
fn a_cancelled_preview_publishes_nothing() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    two_plates(&cli, &stores.ui());
    let ui = stores.ui();
    let before = head(&cli, &ui);
    let mut editor = Editor::default();
    select(&cli, &ui, &mut editor, A);
    editor.preview(EditKind::Delete, target(&ui)).unwrap();
    assert!(editor.can_commit(target(&ui)));
    editor.cancel();
    assert!(editor.begin_commit(target(&ui)).is_none());
    assert_eq!(head(&cli, &ui), before);
    assert_eq!(state(&cli, &ui).len(), 2);
}

#[test]
fn an_engine_refusal_reaches_the_panel_with_its_code_and_publishes_nothing() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    two_plates(&cli, &stores.ui());
    two_plates(&cli, &stores.hand());
    let ui = stores.ui();
    // A translation beyond the coordinate cap is the engine's to refuse.
    let mut editor = Editor::default();
    select(&cli, &ui, &mut editor, A);
    editor.move_to = ["1e12".into(), "0".into(), "0".into()];
    let before = head(&cli, &ui);
    let error = commit(&cli, &mut editor, EditKind::Move, target(&ui)).unwrap_err();
    let (code, hand) = by_hand(
        &cli,
        &[
            "update-element",
            "--store",
            s(&stores.hand()),
            "--id",
            A,
            "--translation",
            "1e12,0,0",
        ],
    );
    assert_eq!(code, 2);
    assert_eq!(error.code, text(&hand, &["code"]));
    assert_eq!(error.message, text(&hand, &["message"]));
    assert_eq!(head(&cli, &ui), before);
    // A weld stops the delete of a welded part (no cascade).
    ok(
        &cli,
        &[
            "create-weld",
            "--store",
            s(&ui),
            "--name",
            "W",
            "--main",
            A,
            "--secondary",
            B,
            "--size",
            "6",
            "--location",
            "workshop",
            "--side",
            "both",
        ],
    );
    let before = head(&cli, &ui);
    select(&cli, &ui, &mut editor, A);
    let error = commit(&cli, &mut editor, EditKind::Delete, target(&ui)).unwrap_err();
    assert_eq!(error.code, "DEPENDENCY_CONFLICT");
    assert_eq!(head(&cli, &ui), before);
}

#[test]
fn undo_and_redo_run_in_a_workspace_and_never_touch_the_published_store() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    let published = stores.hand();
    two_plates(&cli, &published);
    let workspace = stores.workspace();
    ok(
        &cli,
        &[
            "create-workspace",
            "--store",
            s(&published),
            "--workspace-dir",
            s(&workspace),
        ],
    );
    let published_head = head(&cli, &published);
    let published_state = state(&cli, &published);
    let work = Target {
        store: &workspace,
        workspace: Some(&workspace),
    };
    let mut editor = Editor::default();
    // Without a workspace there is nothing to undo.
    assert!(
        editor
            .begin_direct(EditKind::Undo, target(&published))
            .is_err()
    );
    select(&cli, &workspace, &mut editor, A);
    editor.move_to = ["10".into(), "20".into(), "30".into()];
    commit(&cli, &mut editor, EditKind::Move, work).unwrap();
    let moved = state(&cli, &workspace);
    assert_ne!(moved, published_state);
    // Undo restores the earlier state, redo the edit again.
    let undo = editor.begin_direct(EditKind::Undo, work).unwrap().unwrap();
    let undone = command::commit_edit(&cli, &undo, &Stats::default()).unwrap();
    editor.finish();
    assert_eq!(undone.kind, EditKind::Undo);
    assert_eq!(state(&cli, &workspace), published_state);
    let redo = editor.begin_direct(EditKind::Redo, work).unwrap().unwrap();
    command::commit_edit(&cli, &redo, &Stats::default()).unwrap();
    editor.finish();
    assert_eq!(state(&cli, &workspace), moved);
    // Nothing to undo any more after undoing everything: the engine's refusal is shown.
    for _ in 0..3 {
        let undo = editor.begin_direct(EditKind::Undo, work).unwrap().unwrap();
        let result = command::commit_edit(&cli, &undo, &Stats::default());
        editor.finish();
        if let Err(e) = result {
            assert_eq!(e.code, "INVALID_ARGUMENT");
            break;
        }
    }
    // The published store did not move.
    assert_eq!(head(&cli, &published), published_head);
    assert_eq!(state(&cli, &published), published_state);
}

#[test]
fn the_window_binary_refuses_a_store_and_a_workspace_together() {
    let ui = env!("CARGO_BIN_EXE_beamcraft-ui");
    // Both a store and a workspace is a usage error (the workspace is the store).
    let output = Command::new(ui)
        .args(["--store", "S", "--workspace", "W"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).contains("usage: beamcraft-ui"));
    // Without a model the window opens on the start page, so only refused combinations run
    // here: the start page's settings directory and an explicit author belong to different modes.
    // A config directory is now accepted with a store (it names the shortcuts file's directory),
    // but still refused for a metrics run, which has no window to read it in.
    for args in [
        ["--actor-id", "A", "--config-dir", "C"],
        ["--metrics", "1", "--config-dir", "C"],
    ] {
        let output = Command::new(ui).args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
    }
}

/// The no-argument launch is what a double-clicked app bundle does: the default session
/// bootstraps a published store, forks a per-user workspace, and a rerun changes nothing.
#[test]
fn a_launch_without_arguments_bootstraps_a_default_workspace_and_forks_it_once() {
    let Some(cli) = cli() else {
        return;
    };
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("v05-2-tests")
        .join(format!(
            "default-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    let _ = std::fs::remove_dir_all(&root);

    let (workspace, published) =
        source::default_workspace_at(&root, &cli, &Stats::default()).unwrap();
    assert!(workspace.is_dir());
    assert!(published.is_dir());
    let status = [
        "inspect-workspace",
        "--store",
        s(&published),
        "--workspace-dir",
        s(&workspace),
    ];
    let before = format!("{:?}", ok(&cli, &status));

    // A second launch reuses the session: same paths, no second fork, no new revision.
    let again = source::default_workspace_at(&root, &cli, &Stats::default()).unwrap();
    assert_eq!(again, (workspace.clone(), published.clone()));
    assert_eq!(format!("{:?}", ok(&cli, &status)), before);

    let _ = std::fs::remove_dir_all(&root);
}
