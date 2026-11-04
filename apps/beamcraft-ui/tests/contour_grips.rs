// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, MPL-2.0.

//! V18 slices 1 and 2 end to end: the contour grips' commit path — the argument list the window
//! builds for a released drag or an applied corner form and its reply parsing, against the real
//! `beamcraft` binary — saves exactly what the same command typed by hand saves (same contour,
//! same corners, same mesh digest, same world bounds), one revision per action; a refusal (the
//! three-vertex delete, a non-positive treatment) reaches the window with the engine's code and
//! publishes nothing; the selection of a plate carries its authored contour, its corner
//! treatments and the placement a drag maps through; a member carries no contour. The CLI
//! binary must exist next to this package's binary; missing prerequisites fail acceptance.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use beamcraft_json::{Json, Limits};
use beamcraft_ui::command;
use beamcraft_ui::edit::{ContourEdit, PlateContour, parse_contour_edited};
use beamcraft_ui::stats::Stats;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use steel::model::command::CornerTreatment;

static NEXT: AtomicU64 = AtomicU64::new(0);

const ID: &str = "019a7bf0-0000-7000-8000-00000000c001";
const MEMBER: &str = "019a7bf0-0000-7000-8000-00000000c002";

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
            .join("v18-tests")
            .join(format!(
                "run-{}-{}",
                crate::support::boot_nonce(),
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

/// The same pentagon in both stores, with a fixed id: two contours to edit, one through the
/// window's commit path and one by hand.
fn twin_plates(cli: &Path, stores: &Stores) {
    for store in [&stores.ui(), &stores.hand()] {
        ok(
            cli,
            &[
                "create-plate",
                "--store",
                s(store),
                "--contour",
                "0,0;1000,0;1200,300;600,500;0,400",
                "--thickness",
                "10",
                "--new-id",
                ID,
            ],
        );
    }
}

/// What the store holds about the plate: contour, corners, mesh digest and world bounds.
fn plate_state(cli: &Path, store: &Path) -> (Vec<[f64; 3]>, String, String, String) {
    let reply = ok(cli, &["inspect-elements", "--store", s(store)]);
    let plate = at(&reply, &["plates"])
        .array()
        .unwrap()
        .iter()
        .find(|p| text(p, &["element_id"]) == ID)
        .expect("the edited plate is in the reply")
        .clone();
    let contour = at(&plate, &["contour_points_mm"])
        .array()
        .unwrap()
        .iter()
        .map(|p| {
            let triple = p.array().unwrap();
            let value = |j: &Json| j.float().unwrap();
            let [x, y, z] = triple else {
                panic!("a contour point is a triple");
            };
            [value(x), value(y), value(z)]
        })
        .collect::<Vec<_>>();
    let bounds = at(&reply, &["snapshot", "elements"])
        .array()
        .unwrap()
        .iter()
        .find(|e| text(e, &["id"]) == ID)
        .map(|e| format!("{:?}{:?}", e.get("world_min_mm"), e.get("world_max_mm")))
        .unwrap_or_default();
    (
        contour,
        format!("{:?}", plate.get("corners")),
        text(&plate, &["mesh_digest"]),
        bounds,
    )
}

/// The head revision's id.
fn head(cli: &Path, store: &Path) -> String {
    text(
        &ok(cli, &["open-revision", "--store", s(store)]),
        &["revision_id"],
    )
}

/// Runs the window's commit path: the built argument list on the real CLI, then its reply read
/// the way `poll_contour` does.
fn commit_as_window(
    cli: &Path,
    store: &Path,
    edit: &ContourEdit,
) -> Result<beamcraft_ui::edit::ContourEdited, beamcraft_ui::tools::ToolError> {
    let output = command::run(
        cli,
        &edit.args(store, ID),
        command::DEADLINE,
        &Stats::default(),
    );
    match output {
        Ok(output) => parse_contour_edited(edit, output.code, &output.stdout),
        Err(e) => Err(e),
    }
}

#[test]
fn a_drag_commit_saves_what_the_same_hand_typed_command_saves() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    twin_plates(&cli, &stores);
    let before = plate_state(&cli, &stores.ui());
    let hand_before = plate_state(&cli, &stores.hand());
    assert_eq!(before, hand_before, "the twins start equal");

    let drag = ContourEdit::MoveVertex {
        vertex: 1,
        to: [900.0, 80.0],
    };
    let edited = commit_as_window(&cli, &stores.ui(), &drag).expect("the drag commits");
    let (code, _) = by_hand(
        &cli,
        &[
            "move-plate-vertex",
            "--store",
            s(&stores.hand()),
            "--id",
            ID,
            "--vertex",
            "1",
            "--to",
            "900,80",
        ],
    );
    assert_eq!(code, 0);

    assert_eq!(
        plate_state(&cli, &stores.ui()),
        plate_state(&cli, &stores.hand()),
        "the window's drag and the hand-typed command save the same result"
    );
    let sequence: u64 = text(
        &ok(&cli, &["open-revision", "--store", s(&stores.ui())]),
        &["sequence"],
    )
    .parse()
    .unwrap();
    assert_eq!(sequence, edited.sequence, "one revision per drag");
    assert_ne!(
        before,
        plate_state(&cli, &stores.ui()),
        "the drag changed the plate"
    );
}

#[test]
fn an_insert_and_a_delete_run_through_the_same_path() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    twin_plates(&cli, &stores);
    let insert = ContourEdit::InsertVertex {
        edge: 0,
        at: [500.0, 40.0],
    };
    commit_as_window(&cli, &stores.ui(), &insert).expect("the insert commits");
    let (contour, _, _, _) = plate_state(&cli, &stores.ui());
    assert_eq!(contour.len(), 6, "the insert added one vertex");
    assert_eq!(
        contour.get(1),
        Some(&[500.0, 40.0, 0.0]),
        "the inserted vertex sits after edge 0's second end"
    );
    // The inserted vertex sits at position 1; the hand-typed twin must match it.
    let (code, _) = by_hand(
        &cli,
        &[
            "insert-plate-vertex",
            "--store",
            s(&stores.hand()),
            "--id",
            ID,
            "--edge",
            "0",
            "--point",
            "500,40",
        ],
    );
    assert_eq!(code, 0);
    assert_eq!(
        plate_state(&cli, &stores.ui()),
        plate_state(&cli, &stores.hand()),
        "the insert matches the hand-typed one"
    );

    let delete = ContourEdit::DeleteVertex { vertex: 1 };
    commit_as_window(&cli, &stores.ui(), &delete).expect("the delete commits");
    let (contour, _, _, _) = plate_state(&cli, &stores.ui());
    assert_eq!(contour.len(), 5, "the delete removed one vertex");
    assert!(
        !contour.contains(&[500.0, 40.0, 0.0]),
        "the deleted vertex is gone: {contour:?}"
    );
    let (code, _) = by_hand(
        &cli,
        &[
            "delete-plate-vertex",
            "--store",
            s(&stores.hand()),
            "--id",
            ID,
            "--vertex",
            "1",
        ],
    );
    assert_eq!(code, 0);
    assert_eq!(
        plate_state(&cli, &stores.ui()),
        plate_state(&cli, &stores.hand()),
        "delete keeps the twins equal"
    );
}

#[test]
fn a_three_vertex_delete_refuses_and_publishes_nothing() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    // A triangle in both stores: no vertex may leave it.
    for store in [&stores.ui(), &stores.hand()] {
        ok(
            &cli,
            &[
                "create-plate",
                "--store",
                s(store),
                "--contour",
                "0,0;800,0;400,500",
                "--thickness",
                "10",
                "--new-id",
                ID,
            ],
        );
    }
    let before = head(&cli, &stores.ui());
    let edit = ContourEdit::DeleteVertex { vertex: 0 };
    let error = commit_as_window(&cli, &stores.ui(), &edit)
        .expect_err("the CLI refuses the delete below three vertices");
    assert!(
        !error.message.is_empty(),
        "the refusal carries the engine's message"
    );
    assert_eq!(
        head(&cli, &stores.ui()),
        before,
        "a refused drag publishes no revision"
    );
}

#[test]
fn the_selection_carries_the_contour_and_the_placement() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    twin_plates(&cli, &stores);
    let stats = Stats::default();
    let info = command::element_info(&cli, &stores.ui(), ID, &stats).unwrap();
    let contour = info.contour.expect("a flat plate carries its contour");
    assert_eq!(contour.points.len(), 5);
    // The placement the reply reports maps the authored points back onto themselves.
    let placed = contour.placed_points();
    for (authored, world) in contour.points.iter().zip(placed.iter()) {
        assert_eq!(
            contour.local_point(*world),
            Some([authored[0], authored[1]]),
            "a drag mapping through the reported placement keeps the vertex in its plane"
        );
    }
    // A member: no contour, no grips.
    let profile = text(
        &ok(
            &cli,
            &[
                "define-profile",
                "--store",
                s(&stores.ui()),
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
            s(&stores.ui()),
            "--start",
            "0,0,0",
            "--end",
            "3000,0,0",
            "--profile-digest",
            &profile,
            "--new-id",
            MEMBER,
        ],
    );
    let member = command::element_info(&cli, &stores.ui(), MEMBER, &stats).unwrap();
    assert!(
        member.contour.is_none(),
        "a member carries no contour, so it gets no grips"
    );
}

#[test]
fn a_rotated_plate_still_maps_its_drag_onto_its_own_plane() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    ok(
        &cli,
        &[
            "create-plate",
            "--store",
            s(&stores.ui()),
            "--contour",
            "0,0;1000,0;1000,500;0,500",
            "--thickness",
            "10",
            "--orientation-xyzw",
            "0,0.3826834323650898,0,0.9238795325112867",
            "--new-id",
            ID,
        ],
    );
    let info = command::element_info(&cli, &stores.ui(), ID, &Stats::default()).unwrap();
    let contour: PlateContour = info
        .contour
        .expect("a rotated flat plate carries its contour");
    let placed = contour.placed_points();
    for (authored, world) in contour.points.iter().zip(placed.iter()) {
        assert_eq!(
            contour.local_point(*world),
            Some([authored[0], authored[1]]),
            "the drag of the rotated plate stays in its own plane"
        );
    }
}

#[test]
fn an_applied_corner_matches_the_hand_typed_command_and_the_selection_sees_it() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    twin_plates(&cli, &stores);
    let edit = ContourEdit::SetCorner {
        vertex: 0,
        treatment: Some(CornerTreatment::Chamfer {
            d1_mm: 100.0,
            d2_mm: 50.0,
        }),
    };
    commit_as_window(&cli, &stores.ui(), &edit).expect("the applied treatment commits");
    let (code, _) = by_hand(
        &cli,
        &[
            "set-plate-corner",
            "--store",
            s(&stores.hand()),
            "--id",
            ID,
            "--vertex",
            "0",
            "--corner",
            "chamfer:100,50",
        ],
    );
    assert_eq!(code, 0);
    let (_, corners, _, _) = plate_state(&cli, &stores.ui());
    assert!(
        corners.contains("chamfer"),
        "the applied treatment is in the saved corners: {corners}"
    );
    assert_eq!(
        plate_state(&cli, &stores.ui()),
        plate_state(&cli, &stores.hand()),
        "the applied form and the hand-typed command save the same result"
    );
    // The selection carries the treatment, prefilled for the next edit of that corner.
    let contour = command::element_info(&cli, &stores.ui(), ID, &Stats::default())
        .unwrap()
        .contour
        .expect("the plate still carries its contour");
    assert_eq!(
        contour.corner_of(0),
        Some(CornerTreatment::Chamfer {
            d1_mm: 100.0,
            d2_mm: 50.0
        }),
        "the reply's corners round-trip into the editor's prefill"
    );

    // Sharp clears the treatment again, the same way by both paths.
    let sharp = ContourEdit::SetCorner {
        vertex: 0,
        treatment: None,
    };
    commit_as_window(&cli, &stores.ui(), &sharp).expect("the sharp commit clears");
    let (code, _) = by_hand(
        &cli,
        &[
            "set-plate-corner",
            "--store",
            s(&stores.hand()),
            "--id",
            ID,
            "--vertex",
            "0",
            "--corner",
            "sharp",
        ],
    );
    assert_eq!(code, 0);
    let (_, corners, _, _) = plate_state(&cli, &stores.ui());
    assert!(
        corners.contains("[]"),
        "the corner is sharp again: {corners}"
    );
    assert_eq!(
        plate_state(&cli, &stores.ui()),
        plate_state(&cli, &stores.hand()),
        "the clear keeps the twins equal"
    );
}

#[test]
fn a_non_positive_treatment_refuses_and_publishes_nothing() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    twin_plates(&cli, &stores);
    let before = head(&cli, &stores.ui());
    let edit = ContourEdit::SetCorner {
        vertex: 1,
        treatment: Some(CornerTreatment::Round { radius_mm: 0.0 }),
    };
    let error = commit_as_window(&cli, &stores.ui(), &edit)
        .expect_err("the CLI refuses a non-positive treatment");
    assert!(
        !error.message.is_empty(),
        "the refusal carries the engine's message"
    );
    assert_eq!(
        head(&cli, &stores.ui()),
        before,
        "a refused form publishes no revision"
    );
}
