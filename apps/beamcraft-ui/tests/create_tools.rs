// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates and members, MPL-2.0.

//! V05.1 end to end: the create tools' parameter mapping and reply parsing, driven against the
//! real `beamcraft` binary, give exactly what running the CLI by hand gives (same volume, same
//! mesh digest, same world bounds); a cancelled preview publishes nothing; a refusal reaches the
//! panel with the engine's code and message. Without the CLI binary next to this package's binary
//! the tests are skipped and say so (`cargo test --workspace` builds it).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use beamcraft_engine::Representation;
use beamcraft_json::{Json, Limits};
use beamcraft_ui::scene::Scene;
use beamcraft_ui::stats::Stats;
use beamcraft_ui::tools::{MemberKind, PlateForm, ToolKind, Tools};
use beamcraft_ui::{Error, command, source};
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
        eprintln!(
            "SKIP create-tools test: build beamcraft-cli first (cargo test --workspace does)"
        );
        None
    }
}

struct Stores {
    root: PathBuf,
}

impl Stores {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("v05-tests")
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

/// The inspected element (`plates` or `members`, the only one) of a store, run by hand.
fn inspected(cli: &Path, store: &Path, list: &str) -> Json {
    let reply = ok(
        cli,
        &["inspect-elements", "--store", store.to_str().unwrap()],
    );
    let items = at(&reply, &[list]).array().unwrap();
    assert_eq!(items.len(), 1);
    items[0].clone()
}

fn bounds(cli: &Path, store: &Path) -> (Json, Json) {
    let reply = ok(
        cli,
        &["inspect-elements", "--store", store.to_str().unwrap()],
    );
    let element = &at(&reply, &["snapshot", "elements"]).array().unwrap()[0];
    (
        at(element, &["world_min_mm"]).clone(),
        at(element, &["world_max_mm"]).clone(),
    )
}

#[test]
fn a_duplicate_model_name_explains_how_to_recover() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    let config = stores.root.join("config");
    let args = beamcraft_ui::start::project_create_args(
        Some(&config),
        stores.root.to_str().unwrap(),
        "Workshop",
        Some("0199e7a0-0000-7000-8000-0000000000a1"),
    );
    let run = || {
        let output = Command::new(&cli).args(&args).output().unwrap();
        beamcraft_ui::start::parse_project(output.status.code(), &output.stdout, "project_created")
    };
    let created = run().unwrap();
    assert_eq!(created.name, "Workshop");
    let error = run().unwrap_err();
    assert_eq!(error.code, "DUPLICATE_ID");
    assert!(
        error
            .to_string()
            .contains("Choose a different name or location")
    );
    assert!(!error.to_string().contains("DUPLICATE_ID"));
    assert!(stores.root.join("Workshop/revisions").is_dir());
}

#[test]
fn automatic_new_model_names_use_the_same_unique_creation_as_the_cli() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    let config = stores.root.join("config");
    let args = beamcraft_ui::start::project_create_args(
        Some(&config),
        stores.root.to_str().unwrap(),
        "New Model",
        Some("0199e7a0-0000-7000-8000-0000000000a1"),
    );
    for expected in ["New Model", "New Model 1", "New Model 2"] {
        let out = Command::new(&cli)
            .args([
                "suggest-project-name",
                "--parent",
                stores.root.to_str().unwrap(),
                "--name",
                "New Model",
            ])
            .output()
            .unwrap();
        assert_eq!(
            beamcraft_ui::start::parse_project_name(out.status.code(), &out.stdout).unwrap(),
            expected
        );
        let out = Command::new(&cli).args(&args).output().unwrap();
        let created =
            beamcraft_ui::start::parse_project(out.status.code(), &out.stdout, "project_created")
                .unwrap();
        assert_eq!(created.name, expected);
        assert!(Path::new(&created.path).join("revisions").is_dir());
    }
}

#[test]
fn a_plate_from_the_tool_equals_the_plate_made_by_hand() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    let (ui, hand) = (stores.ui(), stores.hand());

    let mut tools = Tools::default();
    tools.plate = PlateForm {
        contour: String::new(),
        class: String::new(),
        width: "1000".into(),
        length: "500".into(),
        thickness: "10".into(),
        name: "Plate A".into(),
        material: None,
    };
    let prepared = tools.preview(&ui).unwrap().clone();
    // Preview and cancel publish nothing: the store has no revision at all.
    tools.cancel();
    assert!(tools.begin_commit(&ui).is_none());
    match source::load(&cli, &ui, &Stats::default()) {
        Err(Error::NoRevision) => (),
        other => panic!("a cancelled preview must publish nothing, got {other:?}"),
    }

    tools.preview(&ui).unwrap();
    assert!(tools.can_commit(&ui));
    let staged = tools.begin_commit(&ui).unwrap();
    assert_eq!(staged, prepared);
    let created = command::commit(&cli, &staged, &Stats::default()).unwrap();
    tools.finish();

    let hand_store = hand.to_str().unwrap();
    let reply = ok(
        &cli,
        &[
            "create-plate",
            "--store",
            hand_store,
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "10",
            "--name",
            "Plate A",
        ],
    );
    assert_eq!(created.kind, ToolKind::Plate);
    assert_eq!(created.mesh_digest, text(&reply, &["plate", "mesh_digest"]));
    assert_eq!(
        created.volume_mm3,
        Some(
            at(&reply, &["plate", "quantities", "volume", "mm3"])
                .float()
                .unwrap()
        )
    );
    assert_eq!(created.volume_mm3, Some(5_000_000.0));
    assert_eq!(created.name, "Plate A");
    assert_eq!(created.sequence, 1);

    // The saved results agree when each store is reopened in a fresh process.
    let (a, b) = (
        inspected(&cli, &ui, "plates"),
        inspected(&cli, &hand, "plates"),
    );
    for path in [
        &["mesh_digest"][..],
        &["quantities", "volume", "mm3"],
        &["quantities", "volume", "algorithm"],
        &["dimensions_mm", "width"],
        &["name"],
    ] {
        assert_eq!(at(&a, path), at(&b, path), "{path:?}");
    }
    assert_eq!(bounds(&cli, &ui), bounds(&cli, &hand));
    assert_eq!(text(&a, &["element_id"]), created.element_id);

    // The viewport reload sees the accepted revision the command reported.
    let snapshot = source::load(&cli, &ui, &Stats::default()).unwrap();
    assert_eq!(
        snapshot.pin().revision().to_string(),
        created.manifest_digest
    );
    let scene = Scene::from_snapshot(&snapshot, Representation::Full).unwrap();
    assert_eq!(scene.elements, 1);
    assert_eq!(scene.counts(), (1, 1));
}

#[test]
fn a_member_from_the_tool_equals_the_member_made_by_hand() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    let (ui, hand) = (stores.ui(), stores.hand());
    for store in [&ui, &hand] {
        let store = store.to_str().unwrap();
        ok(
            &cli,
            &[
                "define-profile",
                "--store",
                store,
                "--width",
                "200",
                "--height",
                "300",
            ],
        );
        ok(
            &cli,
            &[
                "define-material",
                "--store",
                store,
                "--density",
                "7850",
                "--grade",
                "S355",
            ],
        );
    }

    // The choices come from the store's definitions, as the panel lists them: the generated
    // display name (A07.2) leads, the dimensions follow.
    let definitions = command::definitions(&cli, &ui, &Stats::default()).unwrap();
    assert_eq!(definitions.profiles.len(), 1);
    assert_eq!(definitions.materials.len(), 1);
    assert_eq!(
        definitions.profiles[0].label.split(" (").next(),
        Some("RECT 200x300 200 x 300 mm")
    );
    let profile = definitions.profiles[0].digest.clone();
    let material = definitions.materials[0].digest.clone();

    let mut tools = Tools::default();
    tools.active = ToolKind::Member;
    tools.member.kind = MemberKind::Column;
    tools.member.start = ["0".into(), "0".into(), "0".into()];
    tools.member.end = ["0".into(), "0".into(), "3000".into()];
    // T04: the real UI command resolves relative polar input and constrains it to Z.
    tools.member.end_input = "@500<0,3000".into();
    tools.member.axis_lock = beamcraft_engine::coordinate::AxisLock::Z;
    tools.member.profile = Some(profile.clone());
    tools.member.material = Some(material.clone());
    tools.member.name = "C1".into();
    tools.preview(&ui).unwrap();
    let staged = tools.begin_commit(&ui).unwrap();
    let created = command::commit(&cli, &staged, &Stats::default()).unwrap();
    tools.finish();

    let reply = ok(
        &cli,
        &[
            "create-member",
            "--store",
            hand.to_str().unwrap(),
            "--kind",
            "column",
            "--start",
            "0,0,0",
            "--end",
            "0,0,3000",
            "--profile-digest",
            &profile,
            "--name",
            "C1",
            "--material-digest",
            &material,
        ],
    );
    assert_eq!(created.kind, ToolKind::Member);
    assert_eq!(
        created.mesh_digest,
        text(&reply, &["member", "mesh_digest"])
    );
    assert_eq!(
        created.volume_mm3,
        Some(
            at(&reply, &["member", "quantities", "volume", "mm3"])
                .float()
                .unwrap()
        )
    );
    // 200 x 300 mm section over 3000 mm.
    assert_eq!(created.volume_mm3, Some(180_000_000.0));

    let (a, b) = (
        inspected(&cli, &ui, "members"),
        inspected(&cli, &hand, "members"),
    );
    for path in [
        &["mesh_digest"][..],
        &["quantities", "volume", "mm3"],
        &["quantities", "mass", "kg"],
        &["kind"],
        &["profile_digest"],
        &["material_digest"],
    ] {
        assert_eq!(at(&a, path), at(&b, path), "{path:?}");
    }
    assert_eq!(bounds(&cli, &ui), bounds(&cli, &hand));
    let snapshot = source::load(&cli, &ui, &Stats::default()).unwrap();
    assert_eq!(
        snapshot.pin().revision().to_string(),
        created.manifest_digest
    );
    assert_eq!(
        Scene::from_snapshot(&snapshot, Representation::Full)
            .unwrap()
            .elements,
        1
    );
}

#[test]
fn an_engine_refusal_reaches_the_panel_and_publishes_nothing() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    let (ui, hand) = (stores.ui(), stores.hand());
    let mut tools = Tools::default();
    tools.plate = PlateForm {
        contour: String::new(),
        width: "-5".into(),
        length: "500".into(),
        thickness: "10".into(),
        ..PlateForm::default()
    };
    tools.preview(&ui).unwrap();
    let staged = tools.begin_commit(&ui).unwrap();
    let error = command::commit(&cli, &staged, &Stats::default()).unwrap_err();
    tools.finish();
    // The same refusal the CLI prints by hand, code and message.
    let (code, reply) = by_hand(
        &cli,
        &[
            "create-plate",
            "--store",
            hand.to_str().unwrap(),
            "--width",
            "-5",
            "--length",
            "500",
            "--thickness",
            "10",
        ],
    );
    assert_eq!(code, 2);
    assert_eq!(error.code, "INVALID_ARGUMENT");
    assert_eq!(error.code, text(&reply, &["code"]));
    assert_eq!(error.message, text(&reply, &["message"]));
    assert!(matches!(
        source::load(&cli, &ui, &Stats::default()),
        Err(Error::NoRevision)
    ));

    // A member that names a profile the store does not have.
    let mut member = Tools::default();
    member.active = ToolKind::Member;
    member.member.profile = Some("a".repeat(64));
    member.preview(&ui).unwrap();
    let staged = member.begin_commit(&ui).unwrap();
    let error = command::commit(&cli, &staged, &Stats::default()).unwrap_err();
    assert_eq!(error.code, "NOT_FOUND", "{error}");
    assert!(!error.message.is_empty());
}

#[test]
fn a_missing_cli_is_an_error_not_a_hang() {
    let stores = Stores::new();
    let mut tools = Tools::default();
    tools.plate = PlateForm {
        contour: String::new(),
        width: "1".into(),
        length: "1".into(),
        thickness: "1".into(),
        ..PlateForm::default()
    };
    let staged = tools.preview(&stores.ui()).unwrap().clone();
    let error = command::commit(
        &stores.root.join("no-such-beamcraft"),
        &staged,
        &Stats::default(),
    )
    .unwrap_err();
    assert_eq!(error.code, "CLI_UNAVAILABLE");
}

#[test]
fn a_refresh_runs_off_the_ui_thread_and_keeps_its_two_results_apart() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    let ui = stores.ui();
    let refresh = |cli: PathBuf, store: PathBuf| {
        std::thread::spawn(move || {
            source::reload(&cli, &store, &source::Known::default(), &Stats::default())
        })
        .join()
        .unwrap()
    };
    // No revision yet: the scene reports the typed empty state and the definitions listing
    // answers with the CLI's own NOT_FOUND.
    let empty = refresh(cli.clone(), ui.clone());
    assert!(matches!(&empty.scene, Err(Error::NoRevision)));
    assert_eq!(empty.definitions.unwrap_err().code, "NOT_FOUND");
    // The failed export, then the definitions on their own.
    assert_eq!(empty.timing.processes, 2);

    let store = ui.to_str().unwrap();
    ok(
        &cli,
        &[
            "define-profile",
            "--store",
            store,
            "--width",
            "100",
            "--height",
            "200",
        ],
    );
    ok(
        &cli,
        &[
            "create-plate",
            "--store",
            store,
            "--width",
            "10",
            "--length",
            "20",
            "--thickness",
            "5",
        ],
    );
    let full = refresh(cli.clone(), ui.clone());
    let source::Refreshed::New(new) = full.scene.unwrap() else {
        panic!("a first refresh reads a new snapshot");
    };
    let (snapshot, scene) = *new;
    assert_eq!(scene.elements, 1);
    assert!(!snapshot.pin().revision().to_string().is_empty());
    let definitions = full.definitions.unwrap();
    assert_eq!(definitions.profiles.len(), 1);
    assert!(definitions.materials.is_empty());
    // One process and one inspection read the snapshot and the definitions (issue 26).
    assert_eq!(full.timing.processes, 1);
    // The same revision again: nothing to build or upload, and the definitions still arrive.
    let known = source::Known {
        token: Some(snapshot.token().clone()),
        resident: scene.meshes.iter().map(|m| m.digest).collect(),
    };
    let stats = Stats::default();
    let again = std::thread::spawn(move || source::reload(&cli, &ui, &known, &stats))
        .join()
        .unwrap();
    assert!(matches!(again.scene, Ok(source::Refreshed::Unchanged)));
    assert_eq!(again.definitions.unwrap().profiles.len(), 1);
}

#[test]
fn a_snapshot_export_that_misses_its_deadline_is_a_typed_error() {
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
            "10",
            "--length",
            "20",
            "--thickness",
            "5",
        ],
    );
    // A zero deadline cannot be met by a process that has just started: it is stopped.
    match source::load_within(&cli, &ui, std::time::Duration::ZERO, &Stats::default()) {
        Err(Error::Cli(message)) => {
            assert!(message.contains("did not finish in time"), "{message}")
        }
        other => panic!("expected a deadline error, got {other:?}"),
    }
    // The same call within the default deadline succeeds.
    assert!(source::load(&cli, &ui, &Stats::default()).is_ok());
    let missing = source::load(
        &stores.root.join("no-such-beamcraft"),
        &ui,
        &Stats::default(),
    );
    assert!(matches!(missing, Err(Error::Cli(m)) if m.contains("CLI_UNAVAILABLE")));
}

#[test]
fn a_work_plane_plate_from_the_ui_matches_explicit_cli_plane_input() {
    use beamcraft_engine::work_plane::{WorkPlane, pick_work_plane};
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    let (ui, hand) = (stores.ui(), stores.hand());
    let mut tools = Tools::default();
    tools.plate.width = "200".into();
    tools.plate.length = "100".into();
    tools.plate.thickness = "10".into();
    tools.plane = Some(WorkPlane::parallel("xz", 200.).unwrap());
    tools.preview(&ui).unwrap();
    tools.plane = Some(WorkPlane::parallel("yz", 200.).unwrap());
    assert!(
        !tools.can_commit(&ui),
        "a changed session plane invalidates the staged command"
    );
    tools.plane = Some(WorkPlane::parallel("xz", 200.).unwrap());
    let staged = tools.begin_commit(&ui).unwrap();
    command::commit(&cli, &staged, &Stats::default()).unwrap();
    tools.finish();
    ok(
        &cli,
        &[
            "create-plate",
            "--store",
            hand.to_str().unwrap(),
            "--width",
            "200",
            "--length",
            "100",
            "--thickness",
            "10",
            "--plane",
            "0,200,0,1,0,0,0,0,1",
        ],
    );
    let (a, b) = (
        inspected(&cli, &ui, "plates"),
        inspected(&cli, &hand, "plates"),
    );
    for key in ["mesh_digest", "quantities", "placement"] {
        assert_eq!(a.get(key).unwrap(), b.get(key).unwrap(), "{key}");
    }
    assert_eq!(bounds(&cli, &ui), bounds(&cli, &hand));
    let snapshot = source::load(&cli, &ui, &Stats::default()).unwrap();
    let token = snapshot.token().clone();
    let plane = pick_work_plane(&snapshot, [100., 0., 50.], [0., 1., 0.])
        .unwrap()
        .unwrap();
    for (a, b) in plane.normal().into_iter().zip([0., -1., 0.]) {
        assert!((a - b).abs() < 1e-12);
    }
    assert_eq!(
        snapshot.token(),
        &token,
        "face plane selection cannot change the model pin"
    );
}

#[test]
fn picked_concave_plate_in_a_work_plane_matches_the_cli_and_refusal_keeps_the_draft() {
    use beamcraft_engine::work_plane::WorkPlane;
    use beamcraft_ui::creation::ContourDraft;
    let cli = cli().expect("build beamcraft-cli before interactive creation acceptance");
    let stores = Stores::new();
    let plane = WorkPlane::new([1000.0, 2000.0, 3000.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]).unwrap();
    let mut draft = ContourDraft::default();
    for [x, y] in [
        [0.0, 0.0],
        [4000.0, 0.0],
        [4000.0, 1000.0],
        [1000.0, 1000.0],
        [1000.0, 4000.0],
        [0.0, 4000.0],
    ] {
        draft.push(plane.to_world([x, y, 0.0]).unwrap()).unwrap();
    }
    let points = draft.points().to_vec();
    let mut tools = Tools::default();
    tools.plane = Some(plane);
    tools.plate.thickness = "10".into();
    tools.plate.contour = draft.contour(Some(plane)).unwrap();
    tools.preview(&stores.ui()).unwrap();
    tools.cancel();
    assert!(!stores.ui().join("revisions").exists());
    assert_eq!(draft.points(), points);
    tools.preview(&stores.ui()).unwrap();
    let prepared = tools.begin_commit(&stores.ui()).unwrap();
    let created = command::commit(&cli, &prepared, &Stats::default()).unwrap();
    tools.finish();
    assert_eq!(created.volume_mm3, Some(70_000_000.0));
    let direct = ok(
        &cli,
        &[
            "create-plate",
            "--store",
            stores.hand().to_str().unwrap(),
            "--contour",
            "0,0;4000,0;4000,1000;1000,1000;1000,4000;0,4000",
            "--thickness",
            "10",
            "--plane",
            &plane.expression(),
        ],
    );
    assert_eq!(
        created.mesh_digest,
        direct
            .get("plate")
            .unwrap()
            .get("mesh_digest")
            .unwrap()
            .string()
            .unwrap()
    );
    let before = ok(
        &cli,
        &["open-revision", "--store", stores.ui().to_str().unwrap()],
    );
    // Crossing vertices are engine refusals; no repair, publication or draft loss.
    let mut crossed = ContourDraft::default();
    for [x, y] in [[0.0, 0.0], [4000.0, 4000.0], [0.0, 4000.0], [4000.0, 0.0]] {
        crossed.push(plane.to_world([x, y, 0.0]).unwrap()).unwrap();
    }
    tools.plate.contour = crossed.contour(Some(plane)).unwrap();
    let args = tools.preview(&stores.ui()).unwrap().clone();
    assert!(command::commit(&cli, &args, &Stats::default()).is_err());
    assert_eq!(
        before,
        ok(
            &cli,
            &["open-revision", "--store", stores.ui().to_str().unwrap()]
        )
    );
    assert_eq!(crossed.points().len(), 4);
}
