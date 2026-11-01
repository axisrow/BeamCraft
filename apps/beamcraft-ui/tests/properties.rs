// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates and members, MPL-2.0.

//! V06.1 end to end: a property edit made through the properties editor's command building and
//! reply parsing, run against the real `beamcraft` binary, saves exactly what the same
//! `update-element` typed by hand saves; a stale reading is detected against the head the
//! window reads and is never edited; an engine refusal reaches the panel with the CLI's code and
//! publishes nothing; Publish is the CLI's `publish-workspace`, and its refusals are shown.
//! The CLI binary next to this package's binary is required; missing prerequisites fail
//! (`cargo test --workspace` builds it).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use beamcraft_engine::query::pick::SelectionFilter;
use beamcraft_json::{Json, Limits};
use beamcraft_ui::command;
use beamcraft_ui::edit::Ray;
use beamcraft_ui::props::{Properties, State};
use beamcraft_ui::publish::{Phase, Publisher};
use beamcraft_ui::source;
use beamcraft_ui::stats::Stats;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

const PLATE: &str = "019a7bf0-0000-7000-8000-00000000b001";
const BEAM: &str = "019a7bf0-0000-7000-8000-00000000b002";

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
            .join("v06-1-tests")
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

/// A plate and a beam with fixed ids, and the digests of a steel and of a heavier material.
struct Fixture {
    steel: String,
    heavy: String,
}

fn fixture(cli: &Path, store: &Path) -> Fixture {
    let store = s(store);
    let steel = text(
        &ok(
            cli,
            &[
                "define-material",
                "--store",
                store,
                "--density",
                "7850",
                "--grade",
                "S355",
            ],
        ),
        &["definition", "digest"],
    );
    let heavy = text(
        &ok(
            cli,
            &[
                "define-material",
                "--store",
                store,
                "--density",
                "9000",
                "--grade",
                "Heavy",
            ],
        ),
        &["definition", "digest"],
    );
    let profile = text(
        &ok(
            cli,
            &[
                "define-profile",
                "--store",
                store,
                "--width",
                "200",
                "--height",
                "300",
            ],
        ),
        &["definition", "digest"],
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
            "P1",
            "--material-digest",
            &steel,
            "--new-id",
            PLATE,
        ],
    );
    ok(
        cli,
        &[
            "create-member",
            "--store",
            store,
            "--kind",
            "beam",
            "--start",
            "0,0,0",
            "--end",
            "3000,0,0",
            "--profile-digest",
            &profile,
            "--name",
            "B1",
            "--new-id",
            BEAM,
        ],
    );
    Fixture { steel, heavy }
}

/// One element as `inspect-elements` reports it: every property, quantity and digest of it.
fn element(cli: &Path, store: &Path, id: &str) -> String {
    let reply = ok(cli, &["inspect-elements", "--store", s(store)]);
    for key in ["plates", "members"] {
        for e in at(&reply, &[key]).array().unwrap() {
            if text(e, &["element_id"]) == id {
                return format!("{e:?}");
            }
        }
    }
    panic!("element {id} not found");
}

fn head(cli: &Path, store: &Path) -> String {
    text(
        &ok(cli, &["open-revision", "--store", s(store)]),
        &["revision_id"],
    )
}

/// What the panel does: read the details, fill the form, preview, commit.
fn edit_properties(
    cli: &Path,
    store: &Path,
    id: &str,
    fill: impl FnOnce(&mut Properties),
) -> (
    Result<beamcraft_ui::edit::Edited, beamcraft_ui::tools::ToolError>,
    Vec<String>,
) {
    let mut props = Properties::default();
    props.load(
        id,
        command::element_details(cli, store, id, &Stats::default()),
    );
    props.observe_head(
        command::definitions(cli, store, &Stats::default())
            .unwrap()
            .head,
    );
    assert_eq!(props.state(), Some(State::Current));
    fill(&mut props);
    props.preview(store).unwrap();
    let edit = props.begin_commit(store).unwrap();
    let result = command::commit_property(cli, &edit, &Stats::default());
    props.finish();
    (result, edit.args)
}

#[test]
fn a_click_selects_what_is_on_screen_with_one_element_lookup() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    let store = stores.ui();
    fixture(&cli, &store);
    // A pick through the plate's centre from above hits it; a ray into the sky does not. The
    // window's own path is `select`: the engine's pick on the snapshot it shows, then one
    // `inspect-elements --element` for the editor's light report and the panel's full properties.
    let reply = ok(&cli, &["inspect-elements", "--store", s(&store)]);
    let element = &at(&reply, &["snapshot", "elements"]).array().unwrap()[0];
    let corner = |json: &Json| -> [f64; 3] {
        let axis = json.array().unwrap();
        [
            axis[0].float().unwrap(),
            axis[1].float().unwrap(),
            axis[2].float().unwrap(),
        ]
    };
    let min = corner(at(element, &["world_min_mm"]));
    let max = corner(at(element, &["world_max_mm"]));
    let centre = [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0];
    let ray = |origin: [f64; 3], direction: [f64; 3]| Ray { origin, direction };
    let stats = Stats::default();
    let snapshot = source::load(&cli, &store, &stats).unwrap();
    let cancel = AtomicBool::new(false);
    let selected = command::select(
        &cli,
        &store,
        &snapshot,
        &ray([centre[0], centre[1], max[2] + 1000.0], [0.0, 0.0, -1.0]),
        &cancel,
        &stats,
        SelectionFilter::ALL,
    )
    .unwrap()
    .unwrap();
    assert_eq!(selected.id, PLATE);
    let info = selected.info.unwrap();
    assert_eq!(info.name, "P1");
    // The two halves of the lookup read one reply and agree with a separate lookup.
    assert_eq!(
        selected.details,
        command::element_details(&cli, &store, PLATE, &Stats::default())
    );
    assert_eq!(selected.details.unwrap().info(), info);
    // A ray into the sky hits nothing.
    assert!(
        command::select(
            &cli,
            &store,
            &snapshot,
            &ray([centre[0], centre[1], max[2] + 1000.0], [0.0, 0.0, 1.0]),
            &cancel,
            &stats,
            SelectionFilter::ALL,
        )
        .unwrap()
        .is_none()
    );
    // A missing element is NOT_FOUND for both halves of the lookup, for an id no revision can
    // hold alike.
    let gone = command::element_details(
        &cli,
        &store,
        "019a7bf0-0000-7000-8000-0000000000ff",
        &Stats::default(),
    );
    assert_eq!(gone.unwrap_err().code, "NOT_FOUND");
    assert_eq!(
        command::element_info(&cli, &store, "gone", &Stats::default())
            .unwrap_err()
            .code,
        "NOT_FOUND"
    );
}

#[test]
fn a_property_edit_saves_exactly_what_update_element_by_hand_saves() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    let ui = fixture(&cli, &stores.ui());
    let hand = fixture(&cli, &stores.hand());
    assert_eq!((&ui.steel, &ui.heavy), (&hand.steel, &hand.heavy));
    assert_eq!(
        element(&cli, &stores.ui(), PLATE),
        element(&cli, &stores.hand(), PLATE)
    );

    // A plate: every editable property at once.
    let (result, args) = edit_properties(&cli, &stores.ui(), PLATE, |p| {
        p.form.name = "Gusset".into();
        p.form.material = Some(ui.heavy.clone());
        p.form.thickness = "12.5".into();
        p.form.translation = ["100".into(), "-20.5".into(), "300".into()];
    });
    let edited = result.unwrap();
    assert_eq!(edited.element_id.as_deref(), Some(PLATE));
    // The same command typed by hand, on the other store.
    let mut by_hand_args: Vec<&str> = args.iter().map(String::as_str).collect();
    let hand_store = stores.hand();
    *by_hand_args.get_mut(2).unwrap() = s(&hand_store);
    let typed = ok(&cli, &by_hand_args);
    assert_eq!(text(&typed, &["outcome"]), "element_updated");
    assert_eq!(
        element(&cli, &stores.ui(), PLATE),
        element(&cli, &stores.hand(), PLATE)
    );
    assert_eq!(
        edited.sequence.to_string(),
        text(&typed, &["revision", "sequence"])
    );
    // The new values are the engine's, not the form's: read back.
    let details = command::element_details(&cli, &stores.ui(), PLATE, &Stats::default()).unwrap();
    assert_eq!(details.name, "Gusset");
    assert_eq!(details.thickness_mm, Some(12.5));
    assert_eq!(details.translation_mm, [100.0, -20.5, 300.0]);
    assert_eq!(details.material.as_deref(), Some(ui.heavy.as_str()));
    assert_eq!(details.quantities.mass_status.as_deref(), Some("current"));

    // A member: name, material (none to a definition) and position.
    let (result, args) = edit_properties(&cli, &stores.ui(), BEAM, |p| {
        p.form.name = "Beam 7".into();
        p.form.material = Some(ui.steel.clone());
        p.form.translation = ["0".into(), "250".into(), "0".into()];
    });
    result.unwrap();
    let mut by_hand_args: Vec<&str> = args.iter().map(String::as_str).collect();
    *by_hand_args.get_mut(2).unwrap() = s(&hand_store);
    ok(&cli, &by_hand_args);
    assert_eq!(
        element(&cli, &stores.ui(), BEAM),
        element(&cli, &stores.hand(), BEAM)
    );

    // Clearing the material is the engine's explicit `none`.
    let (result, args) = edit_properties(&cli, &stores.ui(), PLATE, |p| p.form.material = None);
    result.unwrap();
    assert!(args.windows(2).any(|w| w == ["--material-digest", "none"]));
    let mut by_hand_args: Vec<&str> = args.iter().map(String::as_str).collect();
    *by_hand_args.get_mut(2).unwrap() = s(&hand_store);
    ok(&cli, &by_hand_args);
    assert_eq!(
        element(&cli, &stores.ui(), PLATE),
        element(&cli, &stores.hand(), PLATE)
    );
}

#[test]
fn a_reading_older_than_the_head_is_stale_and_is_never_edited() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    fixture(&cli, &stores.ui());
    let ui = stores.ui();
    let mut props = Properties::default();
    props.load(
        PLATE,
        command::element_details(&cli, &ui, PLATE, &Stats::default()),
    );
    let listed = command::definitions(&cli, &ui, &Stats::default())
        .unwrap()
        .head
        .unwrap();
    // The reading and the listing name the same revision.
    let read = props.details().unwrap().revision.clone();
    assert_eq!(read, listed);
    props.observe_head(Some(listed));
    assert_eq!(props.state(), Some(State::Current));
    props.form.name = "Late".into();
    props.preview(&ui).unwrap();

    // Someone else edits another element: the head moves, the reading is stale.
    ok(
        &cli,
        &[
            "update-element",
            "--store",
            s(&ui),
            "--id",
            BEAM,
            "--name",
            "Elsewhere",
        ],
    );
    props.observe_head(
        command::definitions(&cli, &ui, &Stats::default())
            .unwrap()
            .head,
    );
    let Some(State::Stale { read, head: newest }) = props.state() else {
        panic!("expected stale");
    };
    assert!(newest.sequence > read.sequence);
    assert!(
        !props.can_commit(&ui),
        "a staged edit is not committed on a stale reading"
    );
    assert!(props.preview(&ui).is_err());
    let before = head(&cli, &ui);

    // Re-reading makes it current again.
    props.load(
        PLATE,
        command::element_details(&cli, &ui, PLATE, &Stats::default()),
    );
    assert_eq!(props.state(), Some(State::Current));
    assert_eq!(head(&cli, &ui), before, "reading publishes nothing");
}

#[test]
fn an_unreadable_element_is_unknown_and_an_engine_refusal_publishes_nothing() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    fixture(&cli, &stores.ui());
    let ui = stores.ui();
    // An id the revision does not hold, or a store that is not readable, is unknown.
    let missing = "019a7bf0-0000-7000-8000-00000000b0ff";
    let mut props = Properties::default();
    props.load(
        missing,
        command::element_details(&cli, &ui, missing, &Stats::default()),
    );
    assert!(matches!(props.state(), Some(State::Unknown { .. })));
    assert!(props.details().is_none());
    assert!(props.prepare(&ui).is_err());
    let nowhere = stores.root.join("nowhere");
    let mut props = Properties::default();
    props.load(
        PLATE,
        command::element_details(&cli, &nowhere, PLATE, &Stats::default()),
    );
    assert!(matches!(props.state(), Some(State::Unknown { .. })));

    // A material the store does not define passes the window's checks and is the engine's refusal.
    let before = head(&cli, &ui);
    let unknown_material = "0".repeat(64);
    let (result, args) = edit_properties(&cli, &ui, PLATE, |p| {
        p.form.material = Some(unknown_material.clone());
    });
    let error = result.unwrap_err();
    let mut by_hand_args: Vec<&str> = args.iter().map(String::as_str).collect();
    let hand = stores.hand();
    fixture(&cli, &hand);
    *by_hand_args.get_mut(2).unwrap() = s(&hand);
    let (code, body) = by_hand(&cli, &by_hand_args);
    assert_ne!(code, 0);
    assert_eq!(error.code, text(&body, &["code"]));
    assert_eq!(error.message, text(&body, &["message"]));
    assert_eq!(head(&cli, &ui), before, "a refused edit publishes nothing");
}

#[test]
fn publish_is_workspace_publish_and_its_refusals_are_shown() {
    let Some(cli) = cli() else { return };
    let stores = Stores::new();
    let published = stores.ui();
    fixture(&cli, &published);
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
    let mut publisher = Publisher::new(Some(published.clone()), Some(workspace.clone()));
    let status =
        command::workspace_status(&cli, &published, &workspace, &Stats::default()).unwrap();
    assert_eq!(status.unsent_revisions, 0);

    // Nothing to publish is the engine's refusal, shown; the dialog ran the command once.
    publisher.request().unwrap();
    let args = publisher.confirm().unwrap();
    let refused = command::publish(&cli, &args, &Stats::default()).unwrap_err();
    assert_eq!(refused.code, "INVALID_ARGUMENT");
    let refused_message = refused.message.clone();
    publisher.finish(Err(refused));
    let shown = publisher.note.clone().unwrap().unwrap_err();
    assert!(shown.contains(&refused_message));
    assert!(!shown.contains("INVALID_ARGUMENT"));
    assert_eq!(publisher.phase(), Phase::Idle);

    // An edit made in the workspace through the properties editor, then published.
    let published_before = element(&cli, &published, PLATE);
    let (result, _) = edit_properties(&cli, &workspace, PLATE, |p| p.form.name = "Edited".into());
    result.unwrap();
    assert_eq!(element(&cli, &published, PLATE), published_before);
    let status =
        command::workspace_status(&cli, &published, &workspace, &Stats::default()).unwrap();
    assert_eq!((status.unsent_revisions, status.base_is_current), (1, true));
    publisher.request().unwrap();
    let args = publisher.confirm().unwrap();
    let done = command::publish(&cli, &args, &Stats::default()).unwrap();
    assert!(!done.replayed);
    publisher.finish(Ok(done.clone()));
    assert!(publisher.note.clone().unwrap().is_ok());
    assert_eq!(
        element(&cli, &published, PLATE),
        element(&cli, &workspace, PLATE),
        "the published element is the workspace's"
    );
    let published_head = ok(&cli, &["open-revision", "--store", s(&published)]);
    assert_eq!(
        text(&published_head, &["revision_id"]),
        done.revision.revision_id
    );

    // The published head moves behind the workspace's back: a second publish is refused and only
    // displayed; nothing is merged and the published head stays where it was.
    let (result, _) = edit_properties(&cli, &workspace, BEAM, |p| p.form.name = "Mine".into());
    result.unwrap();
    ok(
        &cli,
        &[
            "update-element",
            "--store",
            s(&published),
            "--id",
            BEAM,
            "--name",
            "Theirs",
        ],
    );
    let status =
        command::workspace_status(&cli, &published, &workspace, &Stats::default()).unwrap();
    assert!(!status.base_is_current);
    let before = head(&cli, &published);
    publisher.request().unwrap();
    let args = publisher.confirm().unwrap();
    let refused = command::publish(&cli, &args, &Stats::default()).unwrap_err();
    assert_eq!(refused.code, "BASE_REVISION_MISMATCH");
    let refused_message = refused.message.clone();
    publisher.finish(Err(refused));
    let shown = publisher.note.clone().unwrap().unwrap_err();
    assert!(shown.contains(&refused_message));
    assert!(!shown.contains("BASE_REVISION_MISMATCH"));
    assert_eq!(head(&cli, &published), before);
}

#[test]
fn named_level_assignment_changes_only_the_relationship_and_can_be_reset_or_removed() {
    let cli = cli().expect("build beamcraft-cli before the property acceptance suite");
    let stores = Stores::new();
    let store = stores.ui();
    fixture(&cli, &store);
    let level = "01990000-0000-7000-8000-000000007546";
    ok(
        &cli,
        &[
            "create-level",
            "--store",
            s(&store),
            "--new-id",
            level,
            "--name",
            "First floor",
            "--elevation",
            "3500",
        ],
    );
    let before = ok(&cli, &["inspect-elements", "--store", s(&store)]);
    let details = command::element_details(&cli, &store, PLATE, &Stats::default()).unwrap();
    assert_eq!(
        details.levels,
        vec![(level.into(), "First floor".into(), 3500.0)]
    );
    let mut draft = Properties::default();
    draft.load(PLATE, Ok(details.clone()));
    draft.observe_head(Some(details.revision.clone()));
    let pin = head(&cli, &store);
    draft.form.level = Some(level.into());
    draft.load(PLATE, Ok(details)); // Reset reloads values, never executes a command.
    assert_eq!(draft.form.level, None);
    assert_eq!(head(&cli, &store), pin);
    let (assigned, args) = edit_properties(&cli, &store, PLATE, |props| {
        props.form.level = Some(level.into())
    });
    assigned.unwrap();
    assert!(args.windows(2).any(|pair| pair == ["--level", level]));
    let after = ok(&cli, &["inspect-elements", "--store", s(&store)]);
    let plate = |reply: &Json| {
        at(reply, &["plates"])
            .array()
            .unwrap()
            .iter()
            .find(|p| text(p, &["element_id"]) == PLATE)
            .unwrap()
            .clone()
    };
    let old = plate(&before);
    let new = plate(&after);
    assert_eq!(text(&new, &["level"]), level);
    for key in ["mesh_digest", "placement", "stored_quantities"] {
        assert_eq!(
            old.get(key).unwrap(),
            new.get(key).unwrap(),
            "assigning a level must preserve {key}"
        );
    }
    edit_properties(&cli, &store, PLATE, |props| props.form.level = None)
        .0
        .unwrap();
    assert_eq!(
        command::element_details(&cli, &store, PLATE, &Stats::default())
            .unwrap()
            .level,
        None
    );
}
